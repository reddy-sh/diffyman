#!/usr/bin/env python3
"""Compares diffyman's unified diff with `git diff --no-index` and `diff -u`.

For every case in tests/corpus/text the script renders the case with the
`render` example of diffyman-render, runs git and diff with the matching
flags, and reports for each tool whether the output is byte-identical once
header lines are normalized:

- git: the `diff --git`, `index` and mode lines are dropped, and the function
  context after the second `@@` of a hunk header is removed.
- diff: run with `-L a/<left> -L b/<right>`, so its `---`/`+++` lines carry
  no timestamp.

It also applies diffyman's patch to the left file with `patch` and with
`git apply`, and checks that the result equals the right file byte for byte
(only for cases compared without ignore options, where that must hold).

Usage (from the repository root):

    uv run --python 3.12 crates/diffyman-render/tools/compare_unified.py [--verbose]

The script builds the example first. It prints one line per case and a
summary, and exits with status 1 if a patch fails to reproduce the right file
for a reason that is not documented in crates/diffyman-render/README.md.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CORPUS = ROOT / "tests" / "corpus" / "text"
EXAMPLE = ROOT / "target" / "debug" / "examples" / "render"

# Cases where a byte-exact round trip is known not to hold (README, "Known
# differences"): lone CR terminators, BOM-only and invalid UTF-8 inputs.
ROUND_TRIP_EXCEPTIONS = {
    "010-utf8-bom": "a BOM-only difference is not a change (identical document, empty patch)",
    "021-invalid-utf8": "invalid UTF-8 gives an error document (Binary files ... differ)",
    "022-utf16-bom": "UTF-16 input is not UTF-8 (Binary files ... differ)",
    "023-mixed-line-endings": "lone CR lines are written as CR LF",
    "024-lone-cr": "lone CR lines are written as CR LF",
    "025-lone-cr-vs-lf": "lone CR lines are written as CR LF",
}

GIT_ENV = dict(
    os.environ,
    GIT_CONFIG_GLOBAL="/dev/null",
    GIT_CONFIG_NOSYSTEM="1",
    LC_ALL="C",
)


def run(args, cwd, env=None, stdin=None):
    return subprocess.run(
        args, cwd=cwd, env=env, input=stdin, capture_output=True, check=False
    )


def find(case: Path, stem: str) -> Path:
    found = sorted(p for p in case.iterdir() if p.name.startswith(stem + "."))
    if len(found) != 1:
        raise SystemExit(f"{case}: expected one {stem}.* file")
    return found[0]


def flags(options: dict) -> tuple[list[str], list[str], str | None]:
    """Returns (git flags, diff flags, reason the case is not comparable)."""
    ignore = options.get("ignore", {})
    if ignore.get("case"):
        return [], [], "ignore.case has no exact git equivalent"
    if ignore.get("regex"):
        return [], [], "ignore.regex has no git equivalent"
    context = options.get("context", 3)
    algorithm = options.get("algorithm", "histogram")
    git = [f"-U{context}", f"--diff-algorithm={algorithm}"]
    diff = [f"-U{context}"]
    ws = ignore.get("whitespace", "none")
    if ws == "trailing":
        git.append("--ignore-space-at-eol")
        diff = None
    elif ws == "change":
        git.append("-b")
        diff.append("-b")
    elif ws == "all":
        git.append("-w")
        diff.append("-w")
    if ignore.get("blankLines"):
        git.append("--ignore-blank-lines")
        if diff is not None:
            diff.append("-B")
    return git, diff, None


def normalize_git(out: bytes) -> bytes:
    lines = out.split(b"\n")
    kept = []
    in_header = True
    for line in lines:
        if line.startswith(b"@@ "):
            in_header = False
            m = re.match(rb"^(@@ -\S+ \+\S+ @@)", line)
            kept.append(m.group(1) if m else line)
            continue
        if in_header and (
            line.startswith(b"diff --git ")
            or line.startswith(b"index ")
            or line.startswith(b"old mode ")
            or line.startswith(b"new mode ")
        ):
            continue
        kept.append(line)
    return b"\n".join(kept)


def round_trip(left: Path, right: Path, patch: bytes, context: int) -> dict[str, bool]:
    results = {}
    if not patch:
        same = left.read_bytes() == right.read_bytes()
        return {"patch": same, "git apply": same}
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        # BSD patch: apply to a copy of the left file named on the command line.
        target = work / "target"
        shutil.copyfile(left, target)
        (work / "p.diff").write_bytes(patch)
        r = run(["patch", "-s", "-f", "target", "p.diff"], cwd=work)
        results["patch"] = r.returncode == 0 and target.read_bytes() == right.read_bytes()
        # git apply: for a traditional patch whose two names differ, git
        # patches the file named on the +++ line, so the left bytes go there.
        sub = work / "git"
        sub.mkdir()
        shutil.copyfile(left, sub / right.name)
        zero = ["--unidiff-zero"] if context == 0 else []
        r = run(["git", "apply", "-p1", *zero, "../p.diff"], cwd=sub, env=GIT_ENV)
        applied = sub / right.name
        results["git apply"] = (
            r.returncode == 0 and applied.exists() and applied.read_bytes() == right.read_bytes()
        )
    return results


def main() -> int:
    verbose = "--verbose" in sys.argv
    build = run(
        [str(ROOT / "scripts" / "with-rustup"), "cargo", "build", "--quiet",
         "--manifest-path", str(ROOT / "Cargo.toml"), "-p", "diffyman-render",
         "--example", "render"],
        cwd=ROOT,
    )
    if build.returncode != 0:
        sys.stderr.write(build.stderr.decode(errors="replace"))
        return 2

    totals = {"git": 0, "diff": 0, "compared_git": 0, "compared_diff": 0}
    failures = []
    for case in sorted(p for p in CORPUS.iterdir() if p.is_dir()):
        left, right = find(case, "left"), find(case, "right")
        options = json.loads((case / "options.json").read_text())
        ours = run(
            [str(EXAMPLE), "unified", left.name, right.name, "--options", "options.json"],
            cwd=case,
        )
        if ours.returncode != 0:
            message = ours.stderr.decode(errors="replace").strip()
            if "status error" in message:
                print(f"{case.name}: skipped (error document: {message})")
            else:
                failures.append(f"{case.name}: render failed: {message}")
            continue
        git_flags, diff_flags, skip = flags(options)
        cells = []
        if skip:
            cells.append(f"skipped ({skip})")
        else:
            g = run(
                ["git", "-c", "core.quotePath=false", "diff", "--no-index", "--no-color",
                 "--no-ext-diff", "--no-indent-heuristic", *git_flags, left.name, right.name],
                cwd=case, env=GIT_ENV,
            )
            same_git = normalize_git(g.stdout) == ours.stdout
            totals["compared_git"] += 1
            totals["git"] += same_git
            cells.append("git: same" if same_git else "git: DIFFERS")
            if diff_flags is not None:
                d = run(
                    ["diff", "-a", "-u", *diff_flags, "-L", f"a/{left.name}", "-L",
                     f"b/{right.name}", left.name, right.name],
                    cwd=case,
                )
                same_diff = d.stdout == ours.stdout
                totals["compared_diff"] += 1
                totals["diff"] += same_diff
                cells.append("diff -u: same" if same_diff else "diff -u: DIFFERS")
            if verbose and not same_git:
                print(f"--- {case.name}: git (normalized)\n{normalize_git(g.stdout)!r}")
                print(f"--- {case.name}: diffyman\n{ours.stdout!r}")
        ignore = options.get("ignore", {})
        plain = (
            ignore.get("whitespace", "none") == "none"
            and not ignore.get("blankLines")
            and not ignore.get("regex")
            and not ignore.get("case")
        )
        if plain:
            rt = round_trip(case / left.name, case / right.name, ours.stdout,
                            options.get("context", 3))
            ok = all(rt.values())
            cells.append("round trip: ok" if ok else
                         "round trip: FAILED (" + ", ".join(k for k, v in rt.items() if not v) + ")")
            if not ok:
                if case.name in ROUND_TRIP_EXCEPTIONS:
                    cells[-1] += f" [known: {ROUND_TRIP_EXCEPTIONS[case.name]}]"
                else:
                    failures.append(f"{case.name}: patch does not reproduce the right file")
        print(f"{case.name}: " + "; ".join(cells))

    print()
    print(f"git diff --no-index: {totals['git']}/{totals['compared_git']} identical after header normalization")
    print(f"diff -u:             {totals['diff']}/{totals['compared_diff']} identical")
    for f in failures:
        print("FAIL", f)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
