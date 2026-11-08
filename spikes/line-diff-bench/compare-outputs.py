"""Compares two output folders written by `cli-bench.sh`.

    uv run --python 3.12 spikes/line-diff-bench/compare-outputs.py BEFORE_DIR AFTER_DIR

Text outputs must match byte for byte. JSON outputs must match byte for byte
once the top-level `provenance` member (which holds timing) is removed; the
rest of the canonical text is compared as bytes, not re-serialized.
"""

import pathlib
import sys


def strip_provenance(data: bytes) -> bytes:
    """Removes the top-level `"provenance":{...}` member of canonical JSON."""
    key = b',"provenance":{'
    start = data.find(key)
    if start < 0:
        raise ValueError("no provenance member")
    depth = 0
    i = start + len(key) - 1
    in_string = False
    while i < len(data):
        c = data[i : i + 1]
        if in_string:
            if c == b"\\":
                i += 1
            elif c == b'"':
                in_string = False
        elif c == b'"':
            in_string = True
        elif c == b"{":
            depth += 1
        elif c == b"}":
            depth -= 1
            if depth == 0:
                return data[:start] + data[i + 1 :]
        i += 1
    raise ValueError("unterminated provenance member")


def main() -> int:
    before, after = (pathlib.Path(p) for p in sys.argv[1:3])
    failures = 0
    for old in sorted(before.glob("*.out")):
        new = after / old.name
        if not new.exists():
            print(f"missing  {old.name}")
            failures += 1
            continue
        a, b = old.read_bytes(), new.read_bytes()
        if old.name.endswith(".json.out"):
            a, b = strip_provenance(a), strip_provenance(b)
        same = a == b
        failures += not same
        print(f"{'same' if same else 'DIFFERS'}  {old.name}  ({len(b)} bytes)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
