"""Cross-binding conformance: CLI, Python and WASM must return the same bytes.

For every case in tests/corpus/text and tests/corpus/moved this gets the
canonical Diff Document from

  (a) the Rust CLI:      diffyman diff --json --options options.json left.* right.*
  (b) the Python binding: diffyman.diff(Path(left), Path(right), options_text)
  (c) the WASM binding:   node tests/conformance/wasm_diff.mjs --batch

and checks, per docs/spec/diff-document.md section 3, that

  1. each raw output is already RFC 8785 canonical (re-canonicalized with the
     independent Python JCS in jcs.py, it is unchanged),
  2. each output names its own surface in provenance.binding,
  3. after removing the top-level "provenance" key, the canonical bytes are
     identical across all three and identical to expected.json (canonicalized
     the same way), and
  4. the CLI exit code matches the status (0 identical, 1 different, 2 error).

Move detection is deferred in schema 0.1.0, so every moved case is an error
document. Unless --golden-only is given, the moved inputs also run a second
time with "detectMoves": false; those extra cases have no expected.json and
must only agree byte for byte across the three surfaces.

Run from the repository root after building the CLI, the Python binding and the
WASM package (see docs/spikes/B-binding-round-trip.md):

  uv run --no-project --python 3.12 python tests/conformance/run_bindings.py

The Python binding is loaded from the current interpreter if it can import
`diffyman`, otherwise from the virtual environment given by --venv (default
bindings/python/.venv).
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from typing import Any

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import jcs  # noqa: E402

REPO = pathlib.Path(__file__).resolve().parents[2]
CORPUS = REPO / "tests" / "corpus"
GROUPS = ("text", "moved")
SURFACES = ("cli", "python", "wasm")
EXIT_CODES = {"identical": 0, "different": 1, "error": 2}


@dataclass
class Case:
    name: str
    left: pathlib.Path
    right: pathlib.Path
    options: pathlib.Path
    # None for the extra cross-surface-only cases, which have no golden file.
    expected: pathlib.Path | None


@dataclass
class Output:
    raw: str | None = None
    error: str | None = None
    exit_code: int | None = None


@dataclass
class Verdict:
    cells: dict[str, str] = field(default_factory=dict)
    problems: list[str] = field(default_factory=list)

    @property
    def passed(self) -> bool:
        return not self.problems


def only(directory: pathlib.Path, stem: str) -> pathlib.Path:
    found = sorted(p for p in directory.iterdir() if p.is_file() and p.name.startswith(f"{stem}."))
    if len(found) != 1:
        raise SystemExit(f"{directory}: expected exactly one {stem}.* file, found {len(found)}")
    return found[0]


def list_cases(root: pathlib.Path) -> list[Case]:
    cases = []
    for group in GROUPS:
        group_dir = root / group
        if not group_dir.is_dir():
            continue
        for directory in sorted(p for p in group_dir.iterdir() if (p / "options.json").is_file()):
            cases.append(
                Case(
                    name=f"{group}/{directory.name}",
                    left=only(directory, "left"),
                    right=only(directory, "right"),
                    options=directory / "options.json",
                    expected=directory / "expected.json",
                )
            )
    return cases


def line_only_variants(cases: list[Case], workdir: pathlib.Path) -> list[Case]:
    """The moved cases again with "detectMoves": false.

    Move detection is deferred in schema 0.1.0, so every moved case is an
    `unsupported-option` error document. Re-running the same inputs as a plain
    line diff exercises real hunks (CRLF, CJK, right-to-left text) on all three
    surfaces. These have no expected.json; only cross-surface agreement counts.
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
            Case(
                name=f"{case.name} (detectMoves=false)",
                left=case.left,
                right=case.right,
                options=path,
                expected=None,
            )
        )
    return extra


def jobs_for(cases: list[Case]) -> list[dict[str, str]]:
    return [{"left": str(c.left), "right": str(c.right), "options": str(c.options)} for c in cases]


# --- (a) CLI ---------------------------------------------------------------


def find_cli(explicit: str | None) -> pathlib.Path:
    if explicit:
        return pathlib.Path(explicit).resolve()
    exe = "diffyman.exe" if os.name == "nt" else "diffyman"
    candidates = [REPO / "target" / profile / exe for profile in ("release", "debug")]
    existing = [p for p in candidates if p.is_file()]
    if not existing:
        raise SystemExit(
            "no CLI binary found; build it with `cargo build --release -p diffyman-cli` "
            "or pass --cli PATH"
        )
    return max(existing, key=lambda p: p.stat().st_mtime)


def run_cli(cli: pathlib.Path, cases: list[Case]) -> list[Output]:
    outputs = []
    for case in cases:
        proc = subprocess.run(
            [str(cli), "diff", "--json", "--options", str(case.options), str(case.left), str(case.right)],
            capture_output=True,
            check=False,
        )
        out = Output(exit_code=proc.returncode)
        stdout = proc.stdout.decode("utf-8", errors="strict")
        if stdout.endswith("\n") and not stdout.endswith("\n\n"):
            out.raw = stdout[:-1]
        else:
            out.error = f"exit {proc.returncode}: {proc.stderr.decode(errors='replace').strip() or 'no output'}"
        outputs.append(out)
    return outputs


# --- (b) Python ------------------------------------------------------------


def python_worker() -> None:
    """Batch mode run inside the interpreter that has the binding installed."""
    import diffyman

    jobs = json.load(sys.stdin)
    results = []
    for job in jobs:
        try:
            text = pathlib.Path(job["options"]).read_text(encoding="utf-8")
            raw = diffyman.diff(pathlib.Path(job["left"]), pathlib.Path(job["right"]), text)
            results.append({"ok": True, "json": raw})
        except Exception as err:  # noqa: BLE001 - reported per case
            results.append({"ok": False, "error": f"{type(err).__name__}: {err}"})
    sys.stdout.write(json.dumps(results))


def venv_python(venv: pathlib.Path) -> pathlib.Path:
    windows = venv / "Scripts" / "python.exe"
    return windows if windows.is_file() else venv / "bin" / "python"


def run_python(venv: pathlib.Path, cases: list[Case]) -> tuple[list[Output], str]:
    try:
        import diffyman  # noqa: F401

        interpreter = pathlib.Path(sys.executable)
    except ImportError:
        interpreter = venv_python(venv)
        if not interpreter.is_file():
            raise SystemExit(
                f"the diffyman Python binding is not importable and {interpreter} does not exist; "
                "build it with `maturin develop` (bindings/python/README.md) or pass --venv"
            )
    proc = subprocess.run(
        [str(interpreter), str(pathlib.Path(__file__).resolve()), "--python-worker"],
        input=json.dumps(jobs_for(cases)).encode("utf-8"),
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        raise SystemExit(f"python worker failed:\n{proc.stderr.decode(errors='replace')}")
    return batch_outputs(proc.stdout, len(cases)), str(interpreter)


# --- (c) WASM --------------------------------------------------------------


def run_wasm(node: str, pkg: pathlib.Path, cases: list[Case]) -> list[Output]:
    if not (pkg / "diffyman_wasm.js").is_file():
        raise SystemExit(
            f"no WASM package at {pkg}; build it with "
            "`wasm-pack build bindings/wasm --release --target nodejs --out-dir pkg-node`"
        )
    script = REPO / "tests" / "conformance" / "wasm_diff.mjs"
    proc = subprocess.run(
        [node, str(script), "--pkg", str(pkg), "--batch"],
        input=json.dumps(jobs_for(cases)).encode("utf-8"),
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        raise SystemExit(f"wasm_diff.mjs failed:\n{proc.stderr.decode(errors='replace')}")
    return batch_outputs(proc.stdout, len(cases))


def batch_outputs(stdout: bytes, count: int) -> list[Output]:
    results = json.loads(stdout.decode("utf-8"))
    if len(results) != count:
        raise SystemExit(f"batch returned {len(results)} results for {count} cases")
    return [Output(raw=r["json"]) if r["ok"] else Output(error=r["error"]) for r in results]


# --- Comparison ------------------------------------------------------------


def conformance_bytes(document: Any) -> bytes:
    if isinstance(document, dict):
        document = {k: v for k, v in document.items() if k != "provenance"}
    return jcs.canonicalize(document)


def judge(case: Case, outputs: dict[str, Output]) -> Verdict:
    verdict = Verdict()
    expected = None
    if case.expected is not None:
        try:
            expected = conformance_bytes(json.loads(case.expected.read_text(encoding="utf-8")))
        except FileNotFoundError:
            verdict.problems.append("expected.json is missing")

    stripped: dict[str, bytes] = {}
    for surface in SURFACES:
        out = outputs[surface]
        if out.raw is None:
            verdict.cells[surface] = "error"
            verdict.problems.append(f"{surface}: {out.error}")
            continue
        document = json.loads(out.raw)
        cell = "ok"
        if jcs.canonicalize(document) != out.raw.encode("utf-8"):
            cell = "not-jcs"
            verdict.problems.append(f"{surface}: output is not RFC 8785 canonical")
        binding = document.get("provenance", {}).get("binding")
        if binding != surface:
            cell = "bad-prov"
            verdict.problems.append(f"{surface}: provenance.binding is {binding!r}")
        if surface == "cli" and out.exit_code != EXIT_CODES.get(document.get("status")):
            cell = "bad-exit"
            verdict.problems.append(f"cli: exit code {out.exit_code} for status {document.get('status')!r}")
        stripped[surface] = conformance_bytes(document)
        if expected is not None and stripped[surface] != expected:
            cell = "differs"
            verdict.problems.append(f"{surface}: differs from expected.json ({first_difference(expected, stripped[surface])})")
        verdict.cells[surface] = cell

    if len(stripped) == len(SURFACES) and len(set(stripped.values())) != 1:
        verdict.problems.append("the three surfaces disagree with each other")
    verdict.cells["same"] = "yes" if len(stripped) == len(SURFACES) and len(set(stripped.values())) == 1 else "NO"
    return verdict


def first_difference(a: bytes, b: bytes) -> str:
    at = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
    return f"first difference at byte {at}"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--corpus", type=pathlib.Path, default=CORPUS)
    parser.add_argument("--cli", help="path to the diffyman CLI binary (default: newest of target/{release,debug})")
    parser.add_argument("--venv", type=pathlib.Path, default=REPO / "bindings" / "python" / ".venv")
    parser.add_argument("--node", default=shutil.which("node") or "node")
    parser.add_argument("--wasm-pkg", type=pathlib.Path, default=REPO / "bindings" / "wasm" / "pkg-node")
    parser.add_argument(
        "--golden-only",
        action="store_true",
        help="skip the extra moved-inputs-as-line-diff cases that have no expected.json",
    )
    parser.add_argument("--python-worker", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()

    if args.python_worker:
        python_worker()
        return 0

    golden = list_cases(args.corpus.resolve())
    if not golden:
        raise SystemExit(f"no cases under {args.corpus}")
    cli = find_cli(args.cli)
    with tempfile.TemporaryDirectory(prefix="diffyman-bindings-") as tmp:
        extra = [] if args.golden_only else line_only_variants(golden, pathlib.Path(tmp))
        cases = golden + extra
        cli_out = run_cli(cli, cases)
        py_out, interpreter = run_python(args.venv.resolve(), cases)
        wasm_out = run_wasm(args.node, args.wasm_pkg.resolve(), cases)

    print(f"cli:    {cli}")
    print(f"python: {interpreter}")
    print(f"wasm:   {args.wasm_pkg.resolve()} (node {node_version(args.node)})")
    print()
    width = max(len(c.name) for c in cases)
    header = (
        f"{'case':<{width}}  {'status':<9}  {'cli':<8}  {'python':<8}  {'wasm':<8}  "
        f"{'same':<4}  {'golden':<6}  result"
    )
    print(header)
    print("-" * len(header))
    failed = {"golden": 0, "extra": 0}
    for i, case in enumerate(cases):
        outputs = {"cli": cli_out[i], "python": py_out[i], "wasm": wasm_out[i]}
        verdict = judge(case, outputs)
        cells = verdict.cells
        result = "PASS" if verdict.passed else "FAIL"
        status = status_of(outputs["cli"])
        golden_cell = "yes" if case.expected is not None else "-"
        print(
            f"{case.name:<{width}}  {status:<9}  {cells.get('cli', '-'):<8}  "
            f"{cells.get('python', '-'):<8}  {cells.get('wasm', '-'):<8}  "
            f"{cells.get('same', '-'):<4}  {golden_cell:<6}  {result}"
        )
        for problem in verdict.problems:
            print(f"    {problem}")
        failed["golden" if case.expected is not None else "extra"] += not verdict.passed
    print()
    print(
        f"{len(golden)} corpus cases: {len(golden) - failed['golden']} passed, {failed['golden']} failed "
        "(cli, python and wasm byte-identical to expected.json, provenance removed)"
    )
    if extra:
        print(
            f"{len(extra)} extra line-only cases: {len(extra) - failed['extra']} passed, "
            f"{failed['extra']} failed (cli, python and wasm byte-identical to each other)"
        )
    return 1 if failed["golden"] or failed["extra"] else 0


def status_of(out: Output) -> str:
    if out.raw is None:
        return "-"
    return str(json.loads(out.raw).get("status", "-"))


def node_version(node: str) -> str:
    try:
        return subprocess.run([node, "--version"], capture_output=True, check=True, text=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


if __name__ == "__main__":
    sys.exit(main())
