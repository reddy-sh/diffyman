"""Corpus check of the C ABI through the C test program.

For every case in tests/corpus/<group>/<case> this runs the program built
from tests/ffi/test_ffi.c (see tests/ffi/run.sh):

  test_ffi paths left.* right.* options.json   every case
  test_ffi diff  left.* right.* options.json   cases with two files

and checks that each printed document

  1. is already RFC 8785 canonical (re-canonicalized with the independent
     Python JCS in tests/conformance/jcs.py, it is unchanged),
  2. has provenance.binding "c", and
  3. without the top-level "provenance" key equals expected.json,
     canonicalized the same way.

Usage: python3 tests/ffi/check_corpus.py PROGRAM [--corpus DIR]
"""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "tests" / "conformance"))
import jcs  # noqa: E402


def only(directory: pathlib.Path, stem: str) -> pathlib.Path:
    found = sorted(
        p
        for p in directory.iterdir()
        if (p.is_file() and p.name.startswith(f"{stem}.")) or (p.is_dir() and p.name == stem)
    )
    if len(found) != 1:
        raise SystemExit(f"{directory}: expected exactly one {stem}.* file or {stem}/ folder")
    return found[0]


def conformance_bytes(document: object) -> bytes:
    if isinstance(document, dict):
        document = {k: v for k, v in document.items() if k != "provenance"}
    return jcs.canonicalize(document)


def check(program: str, mode: str, case: pathlib.Path, expected: bytes) -> str | None:
    left, right = only(case, "left"), only(case, "right")
    proc = subprocess.run(
        [program, mode, str(left), str(right), str(case / "options.json")],
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        return f"exit {proc.returncode}: {proc.stderr.decode(errors='replace').strip()}"
    raw = proc.stdout
    document = json.loads(raw.decode("utf-8"))
    if jcs.canonicalize(document) != raw:
        return "output is not RFC 8785 canonical"
    binding = document.get("provenance", {}).get("binding")
    if binding != "c":
        return f"provenance.binding is {binding!r}"
    if conformance_bytes(document) != expected:
        return "differs from expected.json"
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("program", help="the program built from tests/ffi/test_ffi.c")
    parser.add_argument("--corpus", type=pathlib.Path, default=REPO / "tests" / "corpus")
    args = parser.parse_args()

    cases = sorted(p.parent for p in args.corpus.resolve().glob("*/*/options.json"))
    if not cases:
        raise SystemExit(f"no cases under {args.corpus}")
    failed = runs = 0
    skipped = []
    for case in cases:
        if not (case / "expected.json").is_file():
            # Not blessed yet; the strict conformance run fails on it.
            skipped.append(f"{case.parent.name}/{case.name}")
            continue
        expected = conformance_bytes(json.loads((case / "expected.json").read_text(encoding="utf-8")))
        folder = only(case, "left").is_dir() or only(case, "right").is_dir()
        for mode in ("paths",) if folder else ("paths", "diff"):
            runs += 1
            problem = check(args.program, mode, case, expected)
            if problem is not None:
                failed += 1
                print(f"FAIL {case.parent.name}/{case.name} ({mode}): {problem}")
    if skipped:
        print(f"no expected.json yet, skipped: {', '.join(skipped)}")
    print(f"{len(cases) - len(skipped)} corpus cases, {runs} runs through the C ABI: {runs - failed} passed, {failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
