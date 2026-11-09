"""Cross-binding conformance: the CLI and every binding must return the same bytes.

For every case in tests/corpus/text, tests/corpus/moved, tests/corpus/binary
and tests/corpus/tree this gets the canonical Diff Document from each surface
(tests/conformance/surfaces.py has the details):

  cli        diffyman diff --json --options options.json left.* right.*
  python     diffyman.diff(Path, Path, text) and diffyman.diff(bytes, bytes, ...)
  wasm       the WASM package (default features), bytes through wasm_diff.mjs
  wasm-slim  the WASM package built without `archives`
  node       diffPaths() and diff() of bindings/node, through node_diff.mjs
  java       Diffyman.diffPaths() and Diffyman.diff(), through JavaDiff.java
  c          diffyman_diff_paths() and diffyman_diff() of the C ABI, through
             the C test program (tests/ffi/run.sh) or ctypes
  server     POST /v1/diff of a running diffyman-server, as JSON (base64)
             and as multipart; only when a server URL is given (--server-url
             or DIFFYMAN_SERVER_URL, with the API key in DIFFYMAN_SERVER_KEY),
             otherwise skipped with a note. tests/server/harness.py starts
             one on a test database and sets both variables.

A binding with both APIs runs twice per case: `paths` (the binding reads the
files or walks the folders) and `bytes` (two files read here and passed with
their final path components as names). Per docs/spec/diff-document.md
section 3, the runner checks that

  1. each raw output is already RFC 8785 canonical (re-canonicalized with the
     independent Python JCS in jcs.py, it is unchanged),
  2. each output names its own surface in provenance.binding,
  3. after removing the top-level "provenance" key, the canonical bytes are
     identical across all surfaces and modes and identical to expected.json
     (canonicalized the same way), and
  4. the CLI exit code matches the status (0 identical, 1 different or
     partial, 2 error).

Cases a surface cannot run are skipped with the reason, never silently: a
folder input has no `bytes` form, and the WASM binding takes bytes only. A
WASM package without the `archives` feature must refuse exactly the cases
spec section 19.7 lists (adapter "tree", a non-empty tree.exclude, or two
inputs that both start like an archive under adapter "auto") with an error
whose code is "unsupported".

The moved cases run with "detectMoves": true. Unless --golden-only is given,
their inputs also run with "detectMoves": false; those extra cases have no
expected.json and must only agree byte for byte across the surfaces.

`--hashes FILE` writes the SHA-256 of every document (provenance removed) per
case, surface and mode as a tab-separated table. Tables written on different
platforms are compared with compare_hashes.py, which is how CI shows the
documents are byte-identical across operating systems.

Run from the repository root after building every surface (README.md in
this folder lists the commands):

  uv run --no-project --python 3.12 python tests/conformance/run_bindings.py

`--surfaces cli,python,...` selects surfaces (default: all) and `--skip`
leaves some out. A selected surface that is not built stops the run with the
command that builds it.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import platform
import sys
import tempfile
from collections import Counter
from dataclasses import dataclass, field
from typing import Any

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import jcs  # noqa: E402
import surfaces as sf  # noqa: E402

REPO = sf.REPO
CORPUS = REPO / "tests" / "corpus"
GROUPS = ("text", "moved", "binary", "tree")
ALL_SURFACES = ("cli", "python", "wasm", "wasm-slim", "node", "java", "c", "server")
EXIT_CODES = {"identical": 0, "different": 1, "partial": 1, "error": 2}


def only(directory: pathlib.Path, stem: str) -> pathlib.Path:
    found = sorted(
        p
        for p in directory.iterdir()
        if (p.is_file() and p.name.startswith(f"{stem}.")) or (p.is_dir() and p.name == stem)
    )
    if len(found) != 1:
        raise SystemExit(f"{directory}: expected exactly one {stem}.* file or {stem}/ folder, found {len(found)}")
    return found[0]


def list_cases(root: pathlib.Path) -> list[sf.Case]:
    cases = []
    for group in GROUPS:
        group_dir = root / group
        if not group_dir.is_dir():
            continue
        for directory in sorted(p for p in group_dir.iterdir() if (p / "options.json").is_file()):
            cases.append(
                sf.Case(
                    name=f"{group}/{directory.name}",
                    left=only(directory, "left"),
                    right=only(directory, "right"),
                    options=directory / "options.json",
                    expected=directory / "expected.json",
                )
            )
    return cases


def line_only_variants(cases: list[sf.Case], workdir: pathlib.Path) -> list[sf.Case]:
    """The moved cases again with "detectMoves": false.

    Re-running the moved inputs as a plain line diff checks, on every surface,
    the same hunks without move detection (CRLF, CJK, right-to-left text).
    These have no expected.json; only cross-surface agreement counts.
    """
    extra = []
    for case in cases:
        if not case.name.startswith("moved/"):
            continue
        options = json.loads(case.options.read_text(encoding="utf-8"))
        if options.get("detectMoves") is not True:
            continue
        options["detectMoves"] = False
        path = workdir / f"{case.name.replace('/', '__')}.options.json"
        path.write_text(json.dumps(options), encoding="utf-8")
        extra.append(
            sf.Case(
                name=f"{case.name} (detectMoves=false)",
                left=case.left,
                right=case.right,
                options=path,
                expected=None,
            )
        )
    return extra


# --- Surfaces ----------------------------------------------------------------


def build_surfaces(args: argparse.Namespace, names: list[str]) -> list[sf.Surface]:
    made: dict[str, sf.Surface | None] = {
        "server": sf.Server(args.server_url) if args.server_url else None,
        "cli": sf.Cli(args.cli),
        "python": sf.Python(args.venv.resolve()),
        "wasm": sf.Wasm(
            "wasm",
            args.node,
            args.wasm_pkg.resolve(),
            "wasm-pack build bindings/wasm --release --target nodejs --out-dir pkg-node",
        ),
        "wasm-slim": sf.Wasm(
            "wasm-slim",
            args.node,
            args.wasm_slim_pkg.resolve(),
            "wasm-pack build bindings/wasm --release --target nodejs --out-dir pkg-node-slim -- --no-default-features",
        ),
        "node": sf.Node(args.node, args.node_pkg.resolve()),
        "java": sf.Java(
            args.java,
            args.java_jar.resolve() if args.java_jar else None,
            args.java_library.resolve() if args.java_library else None,
        ),
        "c": sf.C(
            args.c_driver,
            args.c_program.resolve() if args.c_program else None,
            args.c_library.resolve() if args.c_library else None,
        ),
    }
    surfaces = []
    for name in names:
        surface = made[name]
        if surface is None:
            if args.surfaces:
                raise SystemExit(f"{name}: selected, but no server URL is given (--server-url or DIFFYMAN_SERVER_URL)")
            print(f"{name}: skipped, no server URL given (--server-url or DIFFYMAN_SERVER_URL)")
            continue
        surfaces.append(surface)
    return surfaces


def label(surface: sf.Surface, mode: str) -> str:
    return surface.name if len(surface.modes) == 1 else f"{surface.name}[{mode}]"


def run_all(surfaces: list[sf.Surface], cases: list[sf.Case]) -> dict[tuple[str, str, int], sf.Output]:
    """Every runnable (surface, mode, case) job; skipped jobs are not run."""
    outputs: dict[tuple[str, str, int], sf.Output] = {}
    for surface in surfaces:
        for mode in surface.modes:
            indices = [i for i, case in enumerate(cases) if surface.skip_reason(case, mode) is None]
            results = surface.run([(cases[i], mode) for i in indices])
            for i, out in zip(indices, results, strict=True):
                outputs[(surface.name, mode, i)] = out
    return outputs


# --- Comparison --------------------------------------------------------------


@dataclass
class RunResult:
    surface: str
    mode: str
    binding: str
    # ok, skip, unsupp, error, not-jcs, bad-prov, bad-exit, differs, bad-unsp
    state: str
    sha256: str | None = None
    note: str = ""


@dataclass
class Verdict:
    cells: dict[str, str] = field(default_factory=dict)
    runs: list[RunResult] = field(default_factory=list)
    problems: list[str] = field(default_factory=list)
    golden_sha256: str | None = None
    agreed_sha256: str | None = None

    @property
    def passed(self) -> bool:
        return not self.problems


def conformance_bytes(document: Any) -> bytes:
    if isinstance(document, dict):
        document = {k: v for k, v in document.items() if k != "provenance"}
    return jcs.canonicalize(document)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def judge_run(
    surface: sf.Surface, mode: str, case: sf.Case, out: sf.Output, expected: bytes | None, verdict: Verdict
) -> tuple[RunResult, bytes | None]:
    name = label(surface, mode)
    result = RunResult(surface.name, mode, surface.binding, "ok")
    if surface.refuses(case):
        if out.raw is None and out.code == "unsupported":
            result.state = "unsupp"
            result.note = "refused: no archive support (spec section 19.7)"
        else:
            got = "a document" if out.raw is not None else f"{out.code}: {out.error}"
            result.state = "bad-unsp"
            verdict.problems.append(f"{name}: expected an `unsupported` error (no archive support), got {got}")
        return result, None
    if out.raw is None:
        result.state = "error"
        result.note = f"{out.code}: {out.error}"
        verdict.problems.append(f"{name}: {out.error}")
        return result, None
    document = json.loads(out.raw)
    if jcs.canonicalize(document) != out.raw.encode("utf-8"):
        result.state = "not-jcs"
        verdict.problems.append(f"{name}: output is not RFC 8785 canonical")
    binding = document.get("provenance", {}).get("binding")
    if binding != surface.binding:
        result.state = "bad-prov"
        verdict.problems.append(f"{name}: provenance.binding is {binding!r}, not {surface.binding!r}")
    if surface.name == "cli" and out.exit_code != EXIT_CODES.get(document.get("status")):
        result.state = "bad-exit"
        verdict.problems.append(f"cli: exit code {out.exit_code} for status {document.get('status')!r}")
    stripped = conformance_bytes(document)
    result.sha256 = sha256(stripped)
    if expected is not None and stripped != expected:
        result.state = "differs"
        verdict.problems.append(f"{name}: differs from expected.json ({first_difference(expected, stripped)})")
    return result, stripped


def judge(
    case: sf.Case, index: int, surfaces: list[sf.Surface], outputs: dict[tuple[str, str, int], sf.Output]
) -> Verdict:
    verdict = Verdict()
    expected = None
    if case.expected is not None:
        try:
            expected = conformance_bytes(json.loads(case.expected.read_text(encoding="utf-8")))
            verdict.golden_sha256 = sha256(expected)
        except FileNotFoundError:
            verdict.problems.append("expected.json is missing")

    documents: list[bytes] = []
    complete = True
    for surface in surfaces:
        states = []
        for mode in surface.modes:
            reason = surface.skip_reason(case, mode)
            if reason is not None:
                verdict.runs.append(RunResult(surface.name, mode, surface.binding, "skip", note=reason))
                states.append("skip")
                continue
            result, stripped = judge_run(surface, mode, case, outputs[(surface.name, mode, index)], expected, verdict)
            verdict.runs.append(result)
            states.append(result.state)
            if stripped is not None:
                documents.append(stripped)
            elif result.state != "unsupp":
                complete = False
        ran = [s for s in states if s != "skip"]
        bad = [s for s in ran if s not in ("ok", "unsupp")]
        verdict.cells[surface.name] = bad[0] if bad else ("unsupp" if "unsupp" in ran else "ok") if ran else "skip"

    agree = complete and bool(documents) and len(set(documents)) == 1
    if complete and len(set(documents)) > 1:
        verdict.problems.append("the surfaces disagree with each other")
    verdict.cells["same"] = "yes" if agree else "NO"
    if agree:
        verdict.agreed_sha256 = sha256(documents[0])
    return verdict


def first_difference(a: bytes, b: bytes) -> str:
    at = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
    return f"first difference at byte {at}"


def status_of(index: int, surfaces: list[sf.Surface], outputs: dict[tuple[str, str, int], sf.Output]) -> str:
    """The status of the first document any surface gave for the case."""
    for surface in surfaces:
        for mode in surface.modes:
            out = outputs.get((surface.name, mode, index))
            if out is not None and out.raw is not None:
                return str(json.loads(out.raw).get("status", "-"))
    return "-"


# --- Report ------------------------------------------------------------------


def write_hashes(path: pathlib.Path, surfaces: list[sf.Surface], cases: list[sf.Case], verdicts: list[Verdict]) -> None:
    lines = [
        "# diffyman binding conformance: SHA-256 of each canonical Diff Document with",
        '# the top-level "provenance" key removed (tests/conformance/run_bindings.py).',
        f"# platform: {platform.system()} {platform.machine()}",
        f"# surfaces: {', '.join(s.name for s in surfaces)}",
        "case\tsurface\tmode\tbinding\tresult\tsha256\tnote",
    ]
    for case, verdict in zip(cases, verdicts, strict=True):
        if verdict.golden_sha256 is not None:
            lines.append(f"{case.name}\texpected.json\t-\t-\tgolden\t{verdict.golden_sha256}\t")
        for run in verdict.runs:
            lines.append(
                f"{case.name}\t{run.surface}\t{run.mode}\t{run.binding}\t{run.state}\t{run.sha256 or '-'}\t{run.note}"
            )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")


def parse_names(text: str | None, flag: str) -> list[str]:
    if not text:
        return []
    names = [n.strip() for n in text.split(",") if n.strip()]
    unknown = [n for n in names if n not in ALL_SURFACES]
    if unknown:
        raise SystemExit(f"{flag}: unknown surface {', '.join(unknown)} (known: {', '.join(ALL_SURFACES)})")
    return names


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--corpus", type=pathlib.Path, default=CORPUS)
    parser.add_argument(
        "--surfaces", help=f"comma-separated surfaces to run (default: all of {','.join(ALL_SURFACES)})"
    )
    parser.add_argument("--skip", help="comma-separated surfaces to leave out")
    parser.add_argument("--cli", help="path to the diffyman CLI binary (default: newest of target/{release,debug})")
    parser.add_argument("--venv", type=pathlib.Path, default=REPO / "bindings" / "python" / ".venv")
    parser.add_argument("--node", default=sf.which("node"), help="the Node.js executable (wasm and node)")
    parser.add_argument("--wasm-pkg", type=pathlib.Path, default=REPO / "bindings" / "wasm" / "pkg-node")
    parser.add_argument("--wasm-slim-pkg", type=pathlib.Path, default=REPO / "bindings" / "wasm" / "pkg-node-slim")
    parser.add_argument("--node-pkg", type=pathlib.Path, default=REPO / "bindings" / "node")
    parser.add_argument("--java", default=sf.which("java"), help="the java launcher (JDK 17 or later)")
    parser.add_argument("--java-jar", type=pathlib.Path, help="the binding jar (default: newest bindings/java/target/diffyman-*.jar)")
    parser.add_argument(
        "--java-library", type=pathlib.Path, help="load the JNI library from this file or folder instead of the jar"
    )
    parser.add_argument(
        "--c-driver",
        choices=("auto", "program", "ctypes"),
        default="auto",
        help="C ABI through the compiled test program or ctypes (auto: the program if built, ctypes on Windows)",
    )
    parser.add_argument("--c-program", type=pathlib.Path, help="the program built by tests/ffi/run.sh")
    parser.add_argument("--c-library", type=pathlib.Path, help="the diffyman_ffi shared library (ctypes driver)")
    parser.add_argument(
        "--server-url",
        default=os.environ.get("DIFFYMAN_SERVER_URL"),
        help="a running diffyman-server, http://127.0.0.1:PORT (default: DIFFYMAN_SERVER_URL; the key is read "
        "from DIFFYMAN_SERVER_KEY). Without one the server surface is skipped",
    )
    parser.add_argument("--hashes", type=pathlib.Path, help="write the per-case, per-surface SHA-256 table here")
    parser.add_argument("--quiet", action="store_true", help="print only the failing cases and the summary")
    parser.add_argument(
        "--golden-only",
        action="store_true",
        help="skip the extra moved-inputs-as-line-diff cases that have no expected.json",
    )
    args = parser.parse_args()

    selected = parse_names(args.surfaces, "--surfaces") or list(ALL_SURFACES)
    skipped = set(parse_names(args.skip, "--skip"))
    names = [n for n in ALL_SURFACES if n in selected and n not in skipped]
    if not names:
        raise SystemExit("no surface selected")

    golden = list_cases(args.corpus.resolve())
    if not golden:
        raise SystemExit(f"no cases under {args.corpus}")
    surfaces = build_surfaces(args, names)
    if not surfaces:
        raise SystemExit("no surface left to run")
    for surface in surfaces:
        surface.prepare()

    with tempfile.TemporaryDirectory(prefix="diffyman-bindings-") as tmp:
        extra = [] if args.golden_only else line_only_variants(golden, pathlib.Path(tmp))
        cases = golden + extra
        outputs = run_all(surfaces, cases)
        verdicts = [judge(case, i, surfaces, outputs) for i, case in enumerate(cases)]

    width_name = max(len(s.name) for s in surfaces)
    for surface in surfaces:
        print(f"{surface.name + ':':<{width_name + 1}} {surface.describe()}")
    print(f"platform: {platform.system()} {platform.machine()}, python {platform.python_version()}")
    print()

    width = max(len(c.name) for c in cases)
    cols = [s.name for s in surfaces]
    col_width = max(8, *(len(c) for c in cols))
    header = (
        f"{'case':<{width}}  {'status':<9}  "
        + "  ".join(f"{c:<{col_width}}" for c in cols)
        + f"  {'same':<4}  {'golden':<6}  {'sha256':<16}  result"
    )
    print(header)
    print("-" * len(header))
    failed = {"golden": 0, "extra": 0}
    by_name = {s.name: s for s in surfaces}
    skips: Counter[tuple[str, str]] = Counter()
    refusals: Counter[str] = Counter()
    runs = 0
    for i, (case, verdict) in enumerate(zip(cases, verdicts, strict=True)):
        cells = verdict.cells
        result = "PASS" if verdict.passed else "FAIL"
        status = status_of(i, surfaces, outputs)
        golden_cell = "yes" if case.expected is not None else "-"
        digest = verdict.agreed_sha256[:16] if verdict.agreed_sha256 else "-"
        if not (args.quiet and verdict.passed):
            print(
                f"{case.name:<{width}}  {status:<9}  "
                + "  ".join(f"{cells.get(c, '-'):<{col_width}}" for c in cols)
                + f"  {cells.get('same', '-'):<4}  {golden_cell:<6}  {digest:<16}  {result}"
            )
        for problem in verdict.problems:
            print(f"    {problem}")
        failed["golden" if case.expected is not None else "extra"] += not verdict.passed
        for run in verdict.runs:
            if run.state == "skip":
                skips[(label(by_name[run.surface], run.mode), run.note)] += 1
            else:
                runs += 1
                if run.state == "unsupp":
                    refusals[run.surface] += 1

    print()
    print(
        f"{len(golden)} corpus cases: {len(golden) - failed['golden']} passed, {failed['golden']} failed "
        "(every surface byte-identical to expected.json with provenance removed)"
    )
    if extra:
        print(
            f"{len(extra)} extra line-only cases: {len(extra) - failed['extra']} passed, "
            f"{failed['extra']} failed (every surface byte-identical to the others)"
        )
    print(f"{runs} runs over {len(surfaces)} surfaces ({', '.join(cols)})")
    for refusing, count in sorted(refusals.items()):
        print(f"  {refusing}: {count} cases refused with `unsupported`, as spec section 19.7 requires")
    for (run_label, reason), count in sorted(skips.items()):
        print(f"  skipped {run_label}: {count} cases ({reason})")
    if args.hashes:
        write_hashes(args.hashes.resolve(), surfaces, cases, verdicts)
        print(f"hash table: {args.hashes}")
    return 1 if failed["golden"] or failed["extra"] else 0


if __name__ == "__main__":
    sys.exit(main())
