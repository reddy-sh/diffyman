#!/usr/bin/env python3
"""Re-derive the facts of every tests/corpus/binary/*/expected.json from the inputs.

The engine writes expected.json; this checker backs up the human review with
its own code (diff-document.md sections 6.1, 6.3.1, 6.7, 10, 16 and 17):

- file meta: size, sha256, bom, null line fields, and the media type and
  encoding written by hand in CASES below;
- warnings and status, also from CASES;
- byte ranges: sorted, never empty, `op` consistent with the lengths, gaps of
  equal bytes and equal length on both sides, at least 8 bytes apart, and
  rebuilding the right input from the left one and the ranges;
- stats: byte counts, changeCount, zero line counts and the similarity
  formula;
- the exact ranges for the cases whose answer is known in advance.

Usage (from the repository root):

    uv run --python 3.12 python tests/corpus/tools/check_binary_expected.py
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "binary"

OCTET = "application/octet-stream"

# case: (kind, status, warnings as (code, side), left (mediaType, encoding), right (...),
#        exact ranges as (leftOffset, leftLength, rightOffset, rightLength, op) or None)
CASES = {
    "001-png-one-byte": ("binary", "different", [], ("image/png", None), ("image/png", None),
                         [(200, 1, 200, 1, "replace")]),
    "002-inserted-bytes": ("binary", "different", [], (OCTET, None), (OCTET, None), None),
    "003-truncated": ("binary", "different", [], (OCTET, None), (OCTET, None),
                      [(3000, 1096, 3000, 0, "delete")]),
    "004-identical": ("binary", "identical", [], ("image/png", None), ("image/png", None), []),
    "005-text-vs-binary": ("binary", "different", [("binary-fallback", None)],
                           ("text/plain", "utf-8"), ("image/png", None), None),
    "006-nul-bytes": ("binary", "different", [], (OCTET, None), (OCTET, None),
                      [(17, 1, 17, 1, "replace")]),
    "007-random-1mb-insertion": ("binary", "different", [], (OCTET, None), (OCTET, None), None),
    "008-empty-vs-binary": ("binary", "different", [], (OCTET, "utf-8"), (OCTET, None),
                            [(0, 0, 0, 64, "insert")]),
    "009-latin1-vs-utf8": ("binary", "different", [("binary-fallback", None)],
                           ("text/plain", "utf-8"), (OCTET, None), None),
    "010-utf16-vs-utf8": ("binary", "different", [("binary-fallback", None)],
                          ("text/plain", "utf-8"), ("text/plain", "utf-16le"), None),
    "011-forced-text-on-binary": ("text", "error",
                                  [("invalid-utf8", "left"), ("invalid-utf8", "right")],
                                  ("image/png", None), ("image/png", None), []),
    "012-forced-binary-on-text": ("binary", "different", [], ("text/plain", "utf-8"),
                                  ("text/plain", "utf-8"),
                                  [(6, 4, 6, 4, "replace"), (22, 0, 22, 1, "insert")]),
    "013-chunked-two-edits": ("binary", "different", [], (OCTET, None), (OCTET, None), None),
    "014-close-and-far-changes": ("binary", "different", [], (OCTET, None), (OCTET, None),
                                  [(100, 5, 100, 5, "replace"), (300, 8, 300, 8, "replace"),
                                   (600, 9, 600, 9, "replace"), (900, 1, 900, 1, "replace"),
                                   (909, 1, 909, 1, "replace")]),
    "015-unrelated-coarse": ("binary", "different", [("bytes-coarse-limit", None)],
                             (OCTET, None), (OCTET, None),
                             [(0, 49152, 0, 49152, "replace")]),
    "016-gzip-vs-zip": ("binary", "different", [], ("application/gzip", None),
                        ("application/zip", None), None),
    "017-dense-overwrites": ("binary", "different", [], (OCTET, None), (OCTET, None),
                             [(k * 4228, 8, k * 4228, 8, "replace") for k in range(1, 31)]),
    "018-gif-palette-edit": ("binary", "different", [], ("image/gif", None), ("image/gif", None),
                             [(28, 3, 28, 3, "replace")]),
    "019-pdf-incremental-update": ("binary", "different", [], ("application/pdf", None),
                                   ("application/pdf", None), [(711, 0, 711, 101, "insert")]),
    "020-wasm-section-inserted": ("binary", "different", [], ("application/wasm", None),
                                  ("application/wasm", None), [(15, 0, 15, 15, "insert")]),
    "021-sqlite-page-overwrite": ("binary", "different", [], ("application/vnd.sqlite3", None),
                                  ("application/vnd.sqlite3", None), [(8292, 100, 8292, 100, "replace")]),
    "022-elf-two-patches": ("binary", "different", [], ("application/x-executable", None),
                            ("application/x-executable", None),
                            [(1000, 4, 1000, 4, "replace"), (15000, 4, 15000, 4, "replace")]),
    "023-prepended-header": ("binary", "different", [], (OCTET, None), (OCTET, None),
                             [(0, 0, 0, 64, "insert")]),
    "024-both-empty-forced-binary": ("binary", "identical", [], (OCTET, "utf-8"), (OCTET, "utf-8"), []),
    "025-large-delete-chunked": ("binary", "different", [], (OCTET, None), (OCTET, None), None),
    "026-periodic-delete-canonical": ("binary", "different", [], (OCTET, None), (OCTET, None),
                                      [(4094, 2, 4094, 0, "delete")]),
    "027-binary-vs-empty": ("binary", "different", [], (OCTET, None), (OCTET, "utf-8"),
                            [(0, 300, 0, 0, "delete")]),
    "028-control-bytes-over-limit": ("binary", "different", [], (OCTET, None), (OCTET, None),
                                     [(245, 5, 245, 5, "replace")]),
    "029-extension-fallback-media": ("binary", "different", [], ("image/jpeg", None), ("image/jpeg", None),
                                     [(2000, 1, 2000, 1, "replace")]),
}

# Shapes checked by hand where the exact offsets depend on where a common
# prefix ends: (number of ranges, op of each, right lengths).
SHAPES = {
    "002-inserted-bytes": [("insert", 0, 16)],
    "007-random-1mb-insertion": [("insert", 0, 100)],
    "013-chunked-two-edits": [("insert", 0, 32), ("replace", 40, 40)],
    "025-large-delete-chunked": [("delete", 70000, 0)],
}


class Checker:
    def __init__(self, name: str) -> None:
        self.name = name
        self.problems: list[str] = []

    def eq(self, what: str, actual: object, expected: object) -> None:
        if actual != expected:
            self.problems.append(f"{what}: expected.json has {actual!r}, the inputs give {expected!r}")

    def true(self, what: str, ok: bool) -> None:
        if not ok:
            self.problems.append(what)


def similarity(unchanged: int, left: int, right: int) -> float | int:
    total = left + right
    bp = 10000 if total == 0 else (40000 * unchanged + total) // (2 * total)
    return bp // 10000 if bp % 10000 == 0 else bp / 10000


def check_ranges(c: Checker, doc: dict, left: bytes, right: bytes) -> None:
    rebuilt = bytearray()
    li = ri = 0
    for i, r in enumerate(doc["bytes"]):
        lo, ll, ro, rl, op = r["leftOffset"], r["leftLength"], r["rightOffset"], r["rightLength"], r["op"]
        c.true(f"range {i} is empty", ll > 0 or rl > 0)
        want_op = "replace" if ll and rl else "delete" if ll else "insert"
        c.eq(f"range {i} op", op, want_op)
        c.true(f"range {i} is out of order", lo >= li and ro >= ri)
        c.eq(f"gap lengths before range {i}", lo - li, ro - ri)
        if i:
            c.true(f"range {i} is less than 8 bytes after the previous one", lo - li >= 8)
        c.true(f"gap before range {i} is not equal", left[li:lo] == right[ri:ro])
        rebuilt += left[li:lo] + right[ro : ro + rl]
        li, ri = lo + ll, ro + rl
    c.true("ranges run past the inputs", li <= len(left) and ri <= len(right))
    c.eq("tail lengths", len(left) - li, len(right) - ri)
    c.true("tail is not equal", left[li:] == right[ri:])
    rebuilt += left[li:]
    c.true("the ranges do not rebuild the right input", bytes(rebuilt) == right)


def check_case(directory: Path) -> list[str]:
    c = Checker(directory.name)
    if directory.name not in CASES:
        return [f"{directory.name} is not listed in CASES"]
    kind, status, warnings, left_meta, right_meta, exact = CASES[directory.name]
    doc = json.loads((directory / "expected.json").read_text(encoding="utf-8"))
    options = json.loads((directory / "options.json").read_text(encoding="utf-8"))
    raws = {side: next(directory.glob(f"{side}.*")).read_bytes() for side in ("left", "right")}
    c.eq("schemaVersion", doc["schemaVersion"], "0.4.0")
    c.eq("kind", doc["kind"], kind)
    c.eq("status", doc["status"], status)
    c.eq("options", doc["options"], options)
    c.eq("warnings", [(w["code"], w["side"]) for w in doc["warnings"]], warnings)
    for side, (media, encoding) in (("left", left_meta), ("right", right_meta)):
        meta, raw = doc[side], raws[side]
        c.eq(f"{side}.size", meta["size"], len(raw))
        c.eq(f"{side}.sha256", meta["sha256"], hashlib.sha256(raw).hexdigest())
        c.eq(f"{side}.bom", meta["bom"], raw.startswith(b"\xef\xbb\xbf"))
        c.eq(f"{side}.mediaType", meta["mediaType"], media)
        c.eq(f"{side}.encoding", meta["encoding"], encoding)
        for field in ("lineTerminator", "finalNewline", "lineCount"):
            c.eq(f"{side}.{field}", meta[field], None)
    c.eq("hunks", doc["hunks"], [])
    c.eq("moves", doc["moves"], [])
    if status == "error":
        c.eq("stats", doc["stats"], None)
        c.eq("bytes", doc["bytes"], [])
        return c.problems
    check_ranges(c, doc, raws["left"], raws["right"])
    stats = doc["stats"]
    removed = sum(r["leftLength"] for r in doc["bytes"])
    added = sum(r["rightLength"] for r in doc["bytes"])
    left_size, right_size = len(raws["left"]), len(raws["right"])
    c.eq("stats.bytes", stats["bytes"], {
        "left": left_size, "right": right_size, "added": added, "removed": removed,
        "unchanged": left_size - removed,
    })
    c.eq("unchanged agrees on both sides", left_size - removed, right_size - added)
    c.eq("stats.changeCount", stats["changeCount"], len(doc["bytes"]))
    for field in ("linesLeft", "linesRight", "linesAdded", "linesRemoved", "linesUnchanged", "hunkCount"):
        c.eq(f"stats.{field}", stats[field], 0)
    c.eq("stats.similarity", stats["similarity"], similarity(left_size - removed, left_size, right_size))
    c.eq("identical status", status == "identical", doc["bytes"] == [])
    if exact is not None:
        got = [(r["leftOffset"], r["leftLength"], r["rightOffset"], r["rightLength"], r["op"]) for r in doc["bytes"]]
        c.eq("ranges", got, exact)
    if directory.name in SHAPES:
        got = [(r["op"], r["leftLength"], r["rightLength"]) for r in doc["bytes"]]
        c.eq("range shapes", got, SHAPES[directory.name])
    return c.problems


def main() -> int:
    failed = 0
    cases = sorted(p for p in ROOT.iterdir() if (p / "options.json").is_file())
    for directory in cases:
        if not (directory / "expected.json").is_file():
            print(f"FAIL {directory.name}: no expected.json")
            failed += 1
            continue
        problems = check_case(directory)
        print(f"{'ok  ' if not problems else 'FAIL'} {directory.name}")
        for problem in problems:
            print(f"     {problem}")
        failed += bool(problems)
    missing = sorted(set(CASES) - {p.name for p in cases})
    for name in missing:
        print(f"FAIL {name}: listed in CASES but missing")
    print(f"\n{len(cases)} binary cases checked, {failed} with problems")
    return 1 if failed or missing else 0


if __name__ == "__main__":
    sys.exit(main())
