#!/usr/bin/env python3
"""Generate the text conformance cases under tests/corpus/text/.

Every input file is written byte for byte from the definitions below, so
CRLF terminators, a UTF-8 BOM, a missing final newline and invisible
characters survive exactly. Re-running the script rewrites the inputs,
options.json and case.md of every case; it never touches expected.json.

Usage (from the repository root):

    uv run --python 3.12 python tests/corpus/tools/make_text_cases.py

All content in this file is original and written for the diffyman corpus.
"""

from __future__ import annotations

import json
import sys
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "text"

# The tree options (diff-document.md section 9) at their defaults. They do
# not change a text result; they are spelled out like every other key.
TREE_DEFAULTS = {
    "followSymlinks": False,
    "ignoreFiles": False,
    "exclude": [],
    "detectMoves": True,
    "maxDepth": 3,
    "childBudget": 16777216,
}
MAX_FILE_BYTES = 20 * 1024

BOM = b"\xef\xbb\xbf"


def opts(
    *,
    algorithm: str = "histogram",
    granularity: str = "line",
    whitespace: str = "none",
    case: bool = False,
    blank_lines: bool = False,
    regex: list[str] | None = None,
    context: int = 3,
    position: str = "utf-8",
    adapter: str = "auto",
) -> dict:
    """Return a fully spelled-out options object (diff-document.md section 9).

    Every key is written even when it equals the default, so a case keeps its
    meaning if a default ever changes. detectMoves stays false: the moved/
    cases cover it.
    """
    return {
        "algorithm": algorithm,
        "granularity": granularity,
        "ignore": {
            "whitespace": whitespace,
            "case": case,
            "blankLines": blank_lines,
            "regex": list(regex or []),
        },
        "context": context,
        "positionEncoding": position,
        "detectMoves": False,
        "adapter": adapter,
        "tree": dict(TREE_DEFAULTS),
    }


def lf(*lines: str) -> bytes:
    """Join lines with LF and end the file with LF."""
    return "".join(line + "\n" for line in lines).encode("utf-8")


def crlf(*lines: str) -> bytes:
    """Join lines with CRLF and end the file with CRLF."""
    return "".join(line + "\r\n" for line in lines).encode("utf-8")


def nfd(text: str) -> str:
    return unicodedata.normalize("NFD", text)


def nfc(text: str) -> str:
    return unicodedata.normalize("NFC", text)


CASES: list[dict] = []


def case(
    slug: str,
    ext: str,
    left: bytes,
    right: bytes,
    options: dict,
    about: str,
    *,
    max_bytes: int = MAX_FILE_BYTES,
    valid_utf8: bool = True,
) -> None:
    """Register a case. ``max_bytes`` raises the per-file size limit for the
    one case that needs a very long line; ``valid_utf8=False`` marks the cases
    whose inputs are deliberately not UTF-8."""
    CASES.append(
        {
            "slug": slug,
            "ext": ext,
            "left": left,
            "right": right,
            "options": options,
            "about": about,
            "max_bytes": max_bytes,
            "valid_utf8": valid_utf8,
        }
    )


# 001 -------------------------------------------------------------------------
_identical = lf(
    "Packing list for the lake cabin",
    "",
    "- two sleeping bags",
    "- the blue camping stove and one spare gas canister",
    "- headlamps, with fresh batteries",
    "- a paper map of the north shore trails",
    "- oat bars, coffee, and the small kettle",
)
case(
    "001-identical",
    "txt",
    _identical,
    _identical,
    opts(),
    "Both sides are byte-identical seven-line LF files. A correct result has "
    "`status: \"identical\"`, empty `hunks` and `warnings`, equal `sha256` values on "
    "both sides, `linesUnchanged: 7`, `changeCount: 0`, `hunkCount: 0` and "
    "`similarity: 1`.",
)

# 002 -------------------------------------------------------------------------
case(
    "002-both-empty",
    "txt",
    b"",
    b"",
    opts(),
    "Both inputs are zero bytes. A correct result is `identical` with no hunks and no "
    "warnings. Each side reports `size: 0`, `lineCount: 0`, `finalNewline: false` and "
    "`lineTerminator: \"none\"`, and `similarity` is `1` because L + R = 0 (spec 6.3).",
)

# 003 -------------------------------------------------------------------------
case(
    "003-one-empty",
    "txt",
    b"",
    lf(
        "Meeting moved to Thursday at ten.",
        "Room 4B is booked until noon.",
        "Bring the printed budget draft.",
    ),
    opts(),
    "The left file is empty and the right file has three LF lines. A correct result has "
    "one hunk holding one `insert` op with `leftRange {start: 1, count: 0}` and "
    "`rightRange {start: 1, count: 3}`; the hunk ranges are the same. Stats show "
    "`linesAdded: 3`, `linesUnchanged: 0` and `similarity: 0`. The empty side reports "
    "`lineTerminator: \"none\"` and no `no-newline-at-eof` warning, because it has no "
    "last line.",
)

# 004 -------------------------------------------------------------------------
case(
    "004-simple-line-edit",
    "txt",
    lf(
        "Lentil soup, serves four",
        "1. Rinse 250 g of red lentils.",
        "2. Chop one onion, two carrots and a celery stalk.",
        "3. Soften the vegetables in olive oil for eight minutes.",
        "4. Add the lentils and one litre of vegetable stock.",
        "5. Simmer for twenty minutes, stirring now and then.",
        "6. Season with cumin, salt and black pepper.",
        "7. Blend half of the soup and stir it back in.",
        "8. Finish with lemon juice.",
        "9. Serve with warm flatbread.",
    ),
    lf(
        "Lentil soup, serves four",
        "1. Rinse 250 g of red lentils.",
        "2. Chop one onion, two carrots and a celery stalk.",
        "3. Soften the vegetables in olive oil for eight minutes.",
        "4. Add the lentils and 1.2 litres of vegetable stock.",
        "5. Simmer for twenty minutes, stirring now and then.",
        "6. Season with cumin, salt and black pepper.",
        "7. Blend half of the soup and stir it back in.",
        "8. Finish with lemon juice.",
        "9. Serve with warm flatbread.",
    ),
    opts(algorithm="myers"),
    "Line 5 of a ten-line file changes; nothing else does. The edit is unambiguous, so "
    "every algorithm gives the same script; this case selects `myers` to exercise that "
    "option. A correct result has one hunk `left {2, 7}` / `right {2, 7}` made of equal "
    "L2-4, `replace` L5/R5 and equal L6-8 (three lines of context each way), with "
    "`linesAdded: 1`, `linesRemoved: 1`, `linesUnchanged: 9` and `similarity: 0.9`.",
)

# 005 -------------------------------------------------------------------------
_middle = [
    "alder",
    "birch",
    "cedar",
    "dogwood",
    "elm",
    "fir",
    "ginkgo",
    "hawthorn",
]
case(
    "005-insert-start-end",
    "txt",
    lf(*_middle),
    lf("# Trees on the east path", "", *_middle, "juniper"),
    opts(),
    "Two lines are inserted before line 1 and one line is appended after the last line "
    "of an eight-line file. The eight shared lines exceed 2 x context (6), so a correct "
    "result has two hunks: the first holds an `insert` with `leftRange {start: 1, "
    "count: 0}` and `rightRange {start: 1, count: 2}` followed by three equal lines; "
    "the second holds three equal lines and an `insert` with `leftRange {start: 9, "
    "count: 0}` (N + 1 for an append) and `rightRange {start: 11, count: 1}`.",
)

# 006 -------------------------------------------------------------------------
case(
    "006-delete-block",
    "txt",
    lf(
        "Release checklist",
        "- freeze the main branch",
        "- update the version number",
        "- regenerate the changelog",
        "- run the full test suite",
        "- build the staging images",
        "- deploy to staging",
        "- run the smoke tests on staging",
        "- collect sign-off from support",
        "- collect sign-off from security",
        "- tag the release commit",
        "- publish the packages",
        "- announce the release",
        "- unfreeze the main branch",
        "- archive the build logs",
    ),
    lf(
        "Release checklist",
        "- freeze the main branch",
        "- update the version number",
        "- regenerate the changelog",
        "- run the full test suite",
        "- tag the release commit",
        "- publish the packages",
        "- announce the release",
        "- unfreeze the main branch",
        "- archive the build logs",
    ),
    opts(algorithm="patience", context=2),
    "Five consecutive lines (L6-L10) are deleted from a fifteen-line checklist; every "
    "line is unique, so patience gives the same script as the other algorithms. With "
    "`context: 2` a correct result has one hunk `left {4, 9}` / `right {4, 4}`: equal "
    "L4-5, a `delete` with `leftRange {start: 6, count: 5}` and `rightRange {start: 6, "
    "count: 0}`, then equal L11-12. Stats show `linesRemoved: 5`, `linesUnchanged: 10`.",
)

# 007 -------------------------------------------------------------------------
case(
    "007-whitespace-change",
    "txt",
    lf(
        "# whitespace sample",
        "alpha beta gamma",
        "trailing spaces here",
        "    indented with spaces",
        "form\x0cfeed and\x0bvertical tab",
        "unchanged line one",
        "unchanged line two",
        "foobar baz",
        "unchanged line three",
        "no\u00a0break space",
        "unchanged line four",
        "flush left",
        "end",
    ),
    lf(
        "# whitespace sample",
        "alpha   beta\tgamma",
        "trailing spaces here   \t",
        "\tindented with spaces",
        "form feed and vertical tab",
        "unchanged line one",
        "unchanged line two",
        "foo bar baz",
        "unchanged line three",
        "no break space",
        "unchanged line four",
        "  flush left",
        "end",
    ),
    opts(whitespace="change"),
    "Runs `ignore.whitespace: \"change\"` (git `diff -b`). Lines 2-5 differ only in "
    "whitespace runs, trailing whitespace, tab versus spaces in the indent, and "
    "form feed / vertical tab versus space, so they compare equal. Three lines must "
    "still differ: L8 gains a space where there was none (`foobar` -> `foo bar`), L10 "
    "swaps U+00A0 NO-BREAK SPACE for a plain space (U+00A0 is not whitespace under "
    "spec 7.2), and L12 gains leading whitespace where there was none. A correct "
    "result has one hunk with three single-line `replace` ops at L8, L10 and L12, "
    "`linesUnchanged: 10` and `status: \"different\"`.",
)

# 008 -------------------------------------------------------------------------
_eof_lines = [
    "Station log, week 32",
    "Mon: wind 12 knots, clear",
    "Tue: wind 18 knots, showers",
    "Wed: wind 9 knots, fog until noon",
    "Thu: wind 22 knots, rough sea",
    "Fri: calm, high cloud",
]
case(
    "008-no-final-newline",
    "txt",
    lf(*_eof_lines),
    lf(*_eof_lines)[:-1],
    opts(),
    "Both sides hold the same six lines, but the right file has no LF after its last "
    "line. Under `ignore.whitespace: \"none\"` the terminator kind is part of the "
    "comparison key, so a correct result is `different` with one hunk: equal L3-5 and a "
    "`replace` of L6/R6. The right side has `finalNewline: false`, `lineTerminator: "
    "\"lf\"` (every terminated line uses LF) and a `no-newline-at-eof` warning; the "
    "left side has neither.",
)

# 009 -------------------------------------------------------------------------
_eol_lines = [
    "[general]",
    "name = field-sensor-07",
    "interval_seconds = 300",
    "upload = true",
    "timezone = UTC",
]
case(
    "009-crlf-vs-lf",
    "ini",
    lf(*_eol_lines),
    crlf(*_eol_lines),
    opts(),
    "The same five lines end in LF on the left and CRLF on the right. Under "
    "`ignore.whitespace: \"none\"` the terminator kind is part of the key, so every line "
    "differs: a correct result has one hunk with a single `replace` op covering L1-5 "
    "and R1-5, `linesUnchanged: 0` and `similarity: 0`. The left side reports "
    "`lineTerminator: \"lf\"`, the right side `\"crlf\"`, and both have `finalNewline: "
    "true`, the same `lineCount: 5` and no warnings.",
)

# 010 -------------------------------------------------------------------------
_bom_body = lf(
    "id,city,population",
    "1,Lisbon,545000",
    "2,Porto,232000",
    "3,Braga,193000",
)
case(
    "010-utf8-bom",
    "csv",
    _bom_body,
    BOM + _bom_body,
    opts(),
    "The right file is the left file with a UTF-8 byte order mark (EF BB BF) in front. "
    "The BOM is removed before splitting into lines, so a correct result is "
    "`identical` with no hunks. The right side reports `bom: true` and a `size` three "
    "bytes larger than the left, the two `sha256` values differ, and `warnings` holds "
    "exactly one `bom-stripped` entry with `side: \"right\"`.",
)

# 011 -------------------------------------------------------------------------
case(
    "011-tabs-vs-spaces",
    "conf",
    lf(
        "server {",
        "    listen 8080;",
        "    root /srv/www;",
        "    location / {",
        "        index index.html;",
        "    }",
        "}",
    ),
    lf(
        "server {",
        "\tlisten 8080;",
        "    root /srv/www;\t",
        "    location / {",
        "\t\tindex index.html;",
        "    }",
        "}",
    ),
    opts(whitespace="trailing"),
    "Runs `ignore.whitespace: \"trailing\"`. Line 3 only gains a trailing tab, which "
    "this mode removes, so it compares equal. Lines 2 and 5 swap four or eight leading "
    "spaces for tabs; leading whitespace is not ignored in this mode, so they must "
    "show as changed. A correct result has one hunk with two single-line `replace` ops "
    "(L2/R2 and L5/R5) separated by equal L3-4, `linesUnchanged: 5` and `status: "
    "\"different\"`.",
)

# 012 -------------------------------------------------------------------------
case(
    "012-cjk",
    "txt",
    lf(
        "今天天气很好。",
        "我们去公园散步吧。",
        "日本語のテキストです。",
        "한국어 문장입니다.",
        "全角文字：ＡＢＣ１２３",
        "最后一行。",
    ),
    lf(
        "今天天气很好。",
        "我们去海边散步吧。",
        "日本語のテキストです。",
        "東京は晴れです。",
        "한국어 문장입니다.",
        "全角文字：ＡＢＣ１２３",
        "最后一行。",
    ),
    opts(context=1, position="utf-16"),
    "Chinese, Japanese, Korean and full-width Latin lines. Line 2 changes two Han "
    "characters and a Japanese line is inserted after line 3. With `context: 1` the "
    "single equal line between the two changes is within 2 x context, so a correct "
    "result has one hunk: equal L1, `replace` L2/R2, equal L3, `insert` before L4 "
    "(`rightRange {start: 4, count: 1}`), equal L4/R5. `positionEncoding` is "
    "`utf-16`; with line granularity it only shows up in the echoed options.",
)

# 013 -------------------------------------------------------------------------
case(
    "013-rtl",
    "txt",
    lf(
        "مرحبا بالعالم",
        "هذا سطر عربي ثابت",
        "النسخة 3 جاهزة للتنزيل",
        "שלום עולם",
        "הגרסה 2.4 שוחררה היום",
        "ה-API מוכן לשימוש",
    ),
    lf(
        "مرحبا بالعالم",
        "هذا سطر عربي ثابت",
        "النسخة 4 جاهزة للتنزيل",
        "שלום עולם",
        "הגרסה 2.5 שוחררה היום",
        "ה-API מוכן לשימוש",
    ),
    opts(position="codepoint"),
    "Arabic and Hebrew lines, including digits and a Latin acronym inside right-to-left "
    "text. Only a number changes on line 3 (Arabic) and line 5 (Hebrew). The diff works "
    "on logical (storage) order, never on display order. A correct result has one hunk "
    "covering L1-6 with two single-line `replace` ops at L3 and L5 and equal ops "
    "everywhere else, `linesUnchanged: 4`.",
)

# 014 -------------------------------------------------------------------------
_nf_left = [nfc("café au lait"), nfc("naïve résumé"), nfc("Ångström"), "plain ascii line", nfc("한글")]
_nf_right = [nfd("café au lait"), nfc("naïve résumé"), nfd("Ångström"), "plain ascii line", nfd("한글")]
assert _nf_left[0] != _nf_right[0] and _nf_left[4] != _nf_right[4]
case(
    "014-nfc-vs-nfd",
    "txt",
    lf(*_nf_left),
    lf(*_nf_right),
    opts(position="codepoint"),
    "The left file is in NFC. On the right, lines 1, 3 and 5 are in NFD (e plus U+0301, "
    "A plus U+030A, and Hangul syllables split into conjoining jamo); lines 2 and 4 are "
    "byte-identical. The engine never normalizes (spec 7.1), so the lines look the same "
    "but must still differ: a correct result has one hunk with `replace` ops at L1, L3 "
    "and L5 and equal L2 and L4, `linesUnchanged: 2`, `status: \"different\"`.",
)

# 015 -------------------------------------------------------------------------
case(
    "015-emoji-zwj",
    "txt",
    lf(
        "status: \U0001F469\u200D\U0001F4BB coding",
        "family: \U0001F468\u200D\U0001F469\u200D\U0001F467\u200D\U0001F466",
        "thumbs: \U0001F44D\U0001F3FB",
        "flag: \U0001F1EF\U0001F1F5",
        "plain: ok \u2705",
        "heart: \u2764\uFE0F",
    ),
    lf(
        "status: \U0001F468\u200D\U0001F4BB coding",
        "family: \U0001F468\u200D\U0001F469\u200D\U0001F467\u200D\U0001F466",
        "thumbs: \U0001F44D\U0001F3FF",
        "flag: \U0001F1EF\U0001F1F2",
        "plain: ok \u2705",
        "heart: \u2764",
    ),
    opts(position="utf-16"),
    "Emoji built from several code points: a ZWJ profession sequence, a four-person ZWJ "
    "family, a skin-tone modifier, a regional-indicator flag and a variation selector. "
    "On the right, L1 swaps the woman for a man inside the ZWJ sequence, L3 changes the "
    "skin tone, L4 changes the flag (JP to JM) and L6 drops U+FE0F. L2 and L5 are "
    "unchanged. A correct result has one hunk: `replace` L1, equal L2, `replace` L3-4, "
    "equal L5, `replace` L6; `linesUnchanged: 2`.",
)

# 016 -------------------------------------------------------------------------
case(
    "016-invisible-chars",
    "txt",
    lf(
        "user name: admin",
        "password check passed",
        "spacing  differs   here",
        "attachment: report_final.pdf",
        "total due: 120 EUR",
    ),
    lf(
        "user name: adm\uFEFFin",
        "password\u200B check passed",
        "spacing differs here",
        "attachment: report_\u202Efdp.exe",
        "total due: 120 EUR",
    ),
    opts(whitespace="all", context=5),
    "Runs `ignore.whitespace: \"all\"`, which removes only U+0009, U+000B, U+000C and "
    "U+0020. Line 3 differs only in runs of spaces and compares equal. Lines 1, 2 and 4 "
    "gain invisible characters that are not whitespace and must still differ: U+FEFF "
    "inside a line (not at file start, so it is content, not a BOM), U+200B ZERO WIDTH "
    "SPACE, and U+202E RIGHT-TO-LEFT OVERRIDE disguising a file extension. A correct "
    "result with `context: 5` has one hunk covering the whole file: `replace` L1-2, "
    "equal L3, `replace` L4, equal L5; no `bom-stripped` warning.",
)

# 017 -------------------------------------------------------------------------
case(
    "017-code-refactor",
    "py",
    lf(
        "import csv",
        "",
        "",
        "def summarize(path):",
        "    totals = {}",
        "    with open(path, newline=\"\") as handle:",
        "        for row in csv.DictReader(handle):",
        "            region = row[\"region\"].strip().lower()",
        "            amount = float(row[\"amount\"])",
        "            if region not in totals:",
        "                totals[region] = 0.0",
        "            totals[region] += amount",
        "    lines = []",
        "    for region in sorted(totals):",
        "        lines.append(f\"{region}: {totals[region]:.2f}\")",
        "    return \"\\n\".join(lines)",
        "",
        "",
        "if __name__ == \"__main__\":",
        "    print(summarize(\"sales.csv\"))",
    ),
    lf(
        "import csv",
        "from collections import defaultdict",
        "",
        "",
        "def load_totals(path):",
        "    totals = defaultdict(float)",
        "    with open(path, newline=\"\") as handle:",
        "        for row in csv.DictReader(handle):",
        "            region = row[\"region\"].strip().lower()",
        "            totals[region] += float(row[\"amount\"])",
        "    return totals",
        "",
        "",
        "def summarize(path):",
        "    totals = load_totals(path)",
        "    return \"\\n\".join(",
        "        f\"{region}: {totals[region]:.2f}\" for region in sorted(totals)",
        "    )",
        "",
        "",
        "if __name__ == \"__main__\":",
        "    print(summarize(\"sales.csv\"))",
    ),
    opts(),
    "A small Python refactor: a loader function is extracted, a dictionary becomes a "
    "`defaultdict`, and a loop becomes a generator expression. Several lines (the "
    "`open` and `DictReader` lines, the `region =` line, blank lines) recur in new "
    "positions, so this exercises histogram's choice of anchors on real code. A correct "
    "result is a valid tiling of both files under `text.histogram@1`, with `import csv` "
    "and the final `__main__` block equal, and no move information (`moves: []`).",
)

# 018 -------------------------------------------------------------------------
case(
    "018-log-timestamps-regex",
    "log",
    lf(
        "2026-03-14T08:00:01.120Z INFO  worker-1 started, pid 4120",
        "2026-03-14T08:00:01.348Z INFO  worker-2 started, pid 4121",
        "2026-03-14T08:00:02.007Z INFO  queue connected: jobs-main",
        "2026-03-14T08:00:05.511Z WARN  slow response from inventory (812 ms)",
        "2026-03-14T08:00:09.903Z INFO  batch 1 done: 250 items",
        "2026-03-14T08:00:14.260Z INFO  batch 2 done: 250 items",
        "2026-03-14T08:00:18.775Z INFO  batch 3 done: 250 items",
        "2026-03-14T08:00:19.002Z INFO  shutdown requested",
        "2026-03-14T08:00:19.450Z INFO  worker-1 stopped",
        "2026-03-14T08:00:19.481Z INFO  worker-2 stopped",
    ),
    lf(
        "2026-03-15T08:00:00.981Z INFO  worker-1 started, pid 5307",
        "2026-03-15T08:00:01.204Z INFO  worker-2 started, pid 5308",
        "2026-03-15T08:00:01.866Z INFO  queue connected: jobs-main",
        "2026-03-15T08:00:05.102Z WARN  slow response from inventory (790 ms)",
        "2026-03-15T08:00:09.415Z INFO  batch 1 done: 250 items",
        "2026-03-15T08:00:13.988Z INFO  batch 2 done: 250 items",
        "2026-03-15T08:00:18.390Z ERROR batch 3 failed: inventory timeout",
        "2026-03-15T08:00:18.611Z INFO  shutdown requested",
        "2026-03-15T08:00:19.036Z INFO  worker-1 stopped",
        "2026-03-15T08:00:19.070Z INFO  worker-2 stopped",
    ),
    opts(regex=[r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z"]),
    "Two runs of the same job log; every timestamp differs, and so do the pids, one "
    "latency value and the batch 3 outcome. `ignore.regex` holds one pattern that masks "
    "the leading ISO 8601 timestamp (spec 7.2.1), so lines that differ only in their "
    "timestamp compare equal. A correct result is `different` with one hunk covering "
    "L1-10: `replace` L1-2 (the pids), equal L3, `replace` L4 (the latency), equal L5-6, "
    "`replace` L7 (the batch 3 outcome) and equal L8-10; `linesUnchanged: 6`, "
    "`similarity: 0.6`, and the pattern echoed in `options`.",
)

# 019 -------------------------------------------------------------------------
_block = ["{", "  x = x + 1;", "}", ""]
_left19 = ["// generated counter blocks"]
for _ in range(80):
    _left19.extend(_block)
_right19 = list(_left19)
# Edits, applied from the bottom up so earlier indexes stay valid.
_right19[1 + 70 * 4 + 1] = "  x = x + 2;"  # change the body of block 71
del _right19[1 + 40 * 4 : 1 + 41 * 4]  # delete block 41 entirely
_right19[1 + 20 * 4 + 1] = "  x = x - 1;"  # change the body of block 21
_right19[1 + 5 * 4 : 1 + 5 * 4] = ["{", "  y = y + 1;", "}", ""]  # insert a new block before block 6
case(
    "019-histogram-pathological",
    "txt",
    lf(*_left19),
    lf(*_right19),
    opts(context=0),
    "321 lines made of one header and 80 copies of the same four-line block, so `{`, "
    "`}`, the blank line and `  x = x + 1;` each occur 80 times; no line except the "
    "header is unique, and the occurrence counts are above the chain limit of 64 that "
    "histogram implementations commonly use before falling back. The right side inserts a new block before block 6, changes the bodies of "
    "blocks 21 and 71, and deletes block 41. Many alignments are equally short; the "
    "expected output is whatever `text.histogram@1` produces, so this case pins the "
    "library's tie-breaking and fallback. A correct result must be a valid tiling with "
    "`context: 0` (every change op its own hunk, no equal ops in hunks) and "
    "`linesLeft: 321`, `linesRight: 321`.",
)

# 020 -------------------------------------------------------------------------
case(
    "020-case-insensitive",
    "txt",
    lf(
        "Hello World",
        "ΣΟΦΙΑ",
        "final sigma: ΟΔΟΣ",
        "Kelvin: 300 \u212A",
        "STRAẞE",
        "Fuß",
        "İstanbul",
        "MiXeD CaSe",
        "real change",
    ),
    lf(
        "hello world",
        "σοφια",
        "final sigma: οδος",
        "Kelvin: 300 k",
        "straße",
        "FUSS",
        "istanbul",
        "mixed case",
        "real changes",
    ),
    opts(case=True),
    "Runs `ignore.case: true`, which applies Unicode simple case folding (CaseFolding.txt "
    "statuses C and S only). These pairs fold to the same key and must compare equal: "
    "ASCII case, Greek capital and final sigma, U+212A KELVIN SIGN versus `k`, and "
    "U+1E9E CAPITAL SHARP S versus `ß`. These must still differ: `Fuß` versus `FUSS` "
    "(ß to ss is a full folding, status F), `İstanbul` versus `istanbul` (U+0130 has "
    "only F and T foldings), and a real text change on the last line. A correct result "
    "has one hunk with equal L3-5, `replace` L6-7, equal L8 and `replace` L9 (three "
    "lines of context before the first change), `linesUnchanged: 6`.",
)

# 021 -------------------------------------------------------------------------
case(
    "021-invalid-utf8",
    "txt",
    lf("Invoice 2026-118", "Customer: Harbour Bakery", "Total: 412.50 EUR")[:-1],
    # "Café Lumière" written in Latin-1: 0xE9 and 0xE8 are not valid UTF-8 here.
    b"Invoice 2026-118\nCustomer: Caf\xe9 Lumi\xe8re\nTotal: 412.50 EUR\n",
    opts(adapter="text"),
    "The right file holds two Latin-1 bytes (0xE9 and 0xE8 for `é` and `è`), so it is "
    "not valid UTF-8; the left file is valid UTF-8 with no final newline. The case pins "
    "`adapter: \"text\"`, so the text adapter must read the right file and fails "
    "(spec 7.1; under the default `auto` the pair is compared as bytes instead, see "
    "`binary/009-latin1-vs-utf8`). A correct result is `status: \"error\"` with "
    "`hunks: []`, `moves: []` and `stats: null`. The right side keeps its `size`, "
    "`sha256` and `bom: false`, and its `encoding`, `lineTerminator`, `finalNewline` and "
    "`lineCount` are `null` (spec 8). The left side still decodes, so its meta is filled "
    "in (`lineCount: 3`, `finalNewline: false`). `warnings` is exactly "
    "`no-newline-at-eof` (left) then `invalid-utf8` (right, severity `error`).",
    valid_utf8=False,
)

# 022 -------------------------------------------------------------------------
_utf16_text = "Shopping list\nmilk\neggs\nrye bread\n"
case(
    "022-utf16-bom",
    "txt",
    _utf16_text.encode("utf-8"),
    b"\xff\xfe" + _utf16_text.encode("utf-16-le"),
    opts(adapter="text"),
    "The right file is the left file's text saved as UTF-16LE with its byte order mark "
    "(FF FE). The case pins `adapter: \"text\"` (under `auto` it is compared as bytes, "
    "see `binary/010-utf16-vs-utf8`). Only EF BB BF is treated as a BOM, and a UTF-16 "
    "BOM fails the UTF-8 check (spec 7.1), so a correct result is `status: \"error\"` "
    "with one `invalid-utf8` "
    "error for the right side and no `bom-stripped` warning. The right side reports "
    "`bom: false`, `size: 70` and `null` for `encoding`, `lineTerminator`, "
    "`finalNewline` and `lineCount`; the left side decodes normally (`lineCount: 4`). "
    "`hunks` and `moves` are `[]` and `stats` is `null`.",
    valid_utf8=False,
)

# 023 -------------------------------------------------------------------------
case(
    "023-mixed-line-endings",
    "cfg",
    b"[remote origin]\n"
    b"url = https://example.org/tools/repo.git\n"
    b"fetch = +refs/heads/*\r"
    b"[branch main]\r\n"
    b"remote = origin\r\n"
    b"merge = refs/heads/main\n",
    b"[remote origin]\n"
    b"url = https://example.org/tools/mirror.git\n"
    b"fetch = +refs/heads/*\r"
    b"[branch main]\r\n"
    b"remote = origin\n"
    b"merge = refs/heads/main\n",
    opts(),
    "Both files mix LF, a lone CR (after line 3) and CRLF in one file. The right side "
    "changes the URL on line 2 and ends line 5 with LF instead of CRLF. Each terminator "
    "ends a line, so both sides have `lineCount: 6`, `lineTerminator: \"mixed\"` and a "
    "`mixed-line-endings` warning. Under `ignore.whitespace: \"none\"` the terminator "
    "kind is part of the key, so a correct result has one hunk covering L1-6: equal L1, "
    "`replace` L2, equal L3-4, `replace` L5 (same content, CRLF versus LF), equal L6; "
    "`linesUnchanged: 4` and `similarity: 0.6667`.",
)

# 024 -------------------------------------------------------------------------
_mac_left = ["Tide table, Port Ellen", "06:12 high 3.4 m", "12:31 low 0.8 m", "18:40 high 3.6 m", "", "Times are local."]
_mac_right = ["Tide table, Port Ellen", "06:12 high 3.4 m", "12:29 low 0.7 m", "18:40 high 3.6 m", "", "Times are local."]
case(
    "024-lone-cr",
    "txt",
    "".join(line + "\r" for line in _mac_left).encode("utf-8"),
    "".join(line + "\r" for line in _mac_right).encode("utf-8"),
    opts(),
    "Both files use only a lone CR as the terminator (classic Mac OS style), including "
    "after the empty line 5 and the last line. A lone CR ends a line (spec 7.2), so "
    "both sides have `lineCount: 6`, `lineTerminator: \"cr\"`, `finalNewline: true` and "
    "no warnings. Line 3 changes. A correct result has one hunk: equal L1-2, `replace` "
    "L3/R3, equal L4-6; `linesUnchanged: 5` and `similarity: 0.8333`. A splitter that "
    "treats CR as content would see one line per side instead.",
)

# 025 -------------------------------------------------------------------------
_cr_lines = ["alpha = 1", "beta = 2", "gamma = 3", "delta = 4", "epsilon = 5", "zeta = 6"]
_lf_lines = ["alpha = 1", "beta = 2", "gamma = 3", "delta = 40", "epsilon = 5", "zeta = 6"]
case(
    "025-lone-cr-vs-lf",
    "txt",
    "".join(line + "\r" for line in _cr_lines).encode("utf-8"),
    lf(*_lf_lines),
    opts(whitespace="trailing"),
    "The left file ends every line with a lone CR, the right file with LF, and line 4 "
    "has a real change. Runs `ignore.whitespace: \"trailing\"`, where the terminator is "
    "not part of the key. CR is a terminator, not content (and not whitespace under "
    "spec 7.2), so lines 1-3 and 5-6 compare equal. A correct result has one hunk: "
    "equal L1-3, `replace` L4/R4, equal L5-6; `linesUnchanged: 5`. The left side "
    "reports `lineTerminator: \"cr\"`, the right `\"lf\"`, both `lineCount: 6`, and "
    "there is no `mixed-line-endings` warning.",
)

# 026 -------------------------------------------------------------------------
_blank_left = lf(
    "Chapter 1",
    "",
    "The ferry left at dawn.",
    "Gulls followed it out of the harbour.",
    "",
    "By noon the island was in sight.",
    "",
    "",
    "Nobody spoke on the last mile.",
)
_blank_right = lf(
    "Chapter 1",
    "",
    "",
    "The ferry left at dawn.",
    "Gulls followed it out of the harbour.",
    "",
    "By noon the island was in sight.",
    "",
    "Nobody spoke on the last mile.",
    "",
)
case(
    "026-blank-line-changes",
    "txt",
    _blank_left,
    _blank_right,
    opts(context=1),
    "Only blank lines change: the right side adds a second blank line after the "
    "heading, drops one of the two blank lines before the last paragraph, and appends a "
    "blank line at the end. `ignore.blankLines` is `false`, so every blank-line change "
    "is a real change at line granularity (case 036 runs the same inputs with `true`). "
    "A correct result "
    "with `context: 1` has two hunks. h1: equal L2/R2, an `insert` of R3 "
    "(`leftRange {start: 3, count: 0}`), equal L3/R4. h2: equal L7/R8, a `delete` of "
    "L8, equal L9/R9, and an `insert` appending R10 (`leftRange {start: 10, count: "
    "0}`). Stats show `linesAdded: 2`, `linesRemoved: 1`, `linesUnchanged: 8` and "
    "`similarity: 0.8421`. Which blank line of an equal pair is reported is the "
    "histogram choice; no slider heuristic is applied (spec 7.3).",
)

# 027 -------------------------------------------------------------------------
case(
    "027-only-newline",
    "txt",
    b"\n",
    b"",
    opts(),
    "The left file is a single LF byte; the right file is empty. `\"\\n\"` is one empty "
    "line (spec 7.2), so the left side has `size: 1`, `lineCount: 1`, `finalNewline: "
    "true` and `lineTerminator: \"lf\"`, while the right side has `lineCount: 0`, "
    "`finalNewline: false` and `lineTerminator: \"none\"`. A correct result is "
    "`different` with one hunk holding one `delete` with `leftRange {start: 1, count: "
    "1}` and `rightRange {start: 1, count: 0}`, `linesRemoved: 1`, `similarity: 0` and "
    "no warnings.",
)

# 028 -------------------------------------------------------------------------
_long_words = [f"w{n:05d}" for n in range(1, 28572)]
_long_left = " ".join(_long_words)
_long_right = " ".join("CHANGED" if w == "w14286" else w for w in _long_words)
assert len(_long_left) == 199_996
case(
    "028-long-line",
    "txt",
    lf("# generated word list", _long_left, "# end"),
    lf("# generated word list", _long_right, "# end"),
    opts(granularity="word"),
    "Line 2 is one 199,996-byte line of 28,571 numbered words; the right side changes "
    "the word in the middle. A correct result has one hunk: equal L1, `replace` L2/R2, "
    "equal L3, `linesUnchanged: 2`, `similarity: 0.6667`. The case runs "
    "`granularity: \"word\"`, and each side of the `replace` is far above "
    "`REFINE_MAX_BYTES` (16384, moves spec 3.4), so the op gets `inline: []` and "
    "`warnings` holds exactly one `refine-skipped-limit` entry with `side: null`.",
    max_bytes=256 * 1024,
)

# 029 -------------------------------------------------------------------------
case(
    "029-trailing-whitespace",
    "yaml",
    lf(
        "name: sensor-12",
        "zone: north",
        "   ",
        "mode: active",
        "label: ok\u00a0",
        "note: keep",
        "end",
    )[:-1],
    lf(
        "name: sensor-12   ",
        "zone: north\t",
        "",
        "mode:  active",
        "label: ok",
        "  note: keep",
        "end",
    ),
    opts(whitespace="trailing"),
    "Runs `ignore.whitespace: \"trailing\"`. These must compare equal: L1 gains trailing "
    "spaces, L2 a trailing tab, L3 is three spaces versus an empty line, and L7 has no "
    "final newline on the left only (the terminator is not part of the key in this "
    "mode). These must still differ: L4 widens an inner space, L5 drops a trailing "
    "U+00A0 NO-BREAK SPACE (not whitespace under spec 7.2), and L6 gains leading "
    "spaces. A correct result has one hunk: equal L1-3, one `replace` of L4-6/R4-6, "
    "equal L7; `linesUnchanged: 4`, `status: \"different\"`, and a `no-newline-at-eof` "
    "warning for the left side.",
)

# 030 -------------------------------------------------------------------------
case(
    "030-tabs-whitespace-change",
    "txt",
    lf(
        "PATH\t=\t/usr/bin",
        "\tindent with tab",
        "cols:\ta\t\tb",
        "trail\t",
        "ab",
        "x",
        "same",
    ),
    lf(
        "PATH = /usr/bin",
        "    indent with tab",
        "cols: a b",
        "trail",
        "a\tb",
        "\tx",
        "same",
    ),
    opts(whitespace="change"),
    "Runs `ignore.whitespace: \"change\"` on tab-heavy lines. Every run of tabs and "
    "spaces collapses to one space and trailing whitespace is dropped, so L1 (tabs "
    "around `=`), L2 (tab versus four spaces of indent), L3 (one and two tabs versus "
    "single spaces) and L4 (trailing tab) compare equal. L5 gains a tab where there was "
    "no whitespace and L6 gains a leading tab, so both must differ. A correct result "
    "has one hunk: equal L2-4, one `replace` of L5-6/R5-6, equal L7; `linesUnchanged: "
    "5`, `similarity: 0.7143`.",
)

# 031 -------------------------------------------------------------------------
case(
    "031-utf16-word-cjk-emoji",
    "txt",
    lf(
        "今日は\U0001F363を食べた",
        "status: \U0001F44D\U0001F3FD great",
        "東京 2026 \U0001F5FC trip",
    ),
    lf(
        "今日は\U0001F35Cを食べた",
        "status: \U0001F44D\U0001F3FF great",
        "大阪 2026 \U0001F3EF trip",
    ),
    opts(granularity="word", position="utf-16"),
    "Runs `granularity: \"word\"` with `positionEncoding: \"utf-16\"` on Japanese text "
    "and emoji. All three lines change, so there is one `replace` of L1-3/R1-3 with "
    "spans. UAX #29 makes each ideograph and hiragana its own word segment and keeps an "
    "emoji with its skin-tone modifier together. A correct result has spans, in UTF-16 "
    "code units: L1/R1 `[3, 5)` (the sushi and ramen emoji, two units each); L2/R2 "
    "`[8, 12)` (thumbs up plus a skin tone, four units); L3/R3 `[0, 2)` (`東京` / "
    "`大阪`, one merged span of two segments) and `[8, 10)` (the tower and castle "
    "emoji). Left spans have `op: \"delete\"`, right spans `op: \"insert\"`.",
)

# 032 -------------------------------------------------------------------------
case(
    "032-codepoint-char-combining",
    "txt",
    lf(
        "cafe\u0301 noir",
        "ma\u00f1ana",
        "x\u0323\u0307 = 1",
    ),
    lf(
        "cafe\u0300 noir",
        "man\u0303ana",
        "x\u0307 = 1",
    ),
    opts(granularity="char", position="codepoint"),
    "Runs `granularity: \"char\"` with `positionEncoding: \"codepoint\"` on combining "
    "marks. Tokens are extended grapheme clusters, so a span never splits a base letter "
    "from its marks. L1 swaps U+0301 for U+0300 after `e`: spans `[3, 5)` on both "
    "sides. L2 is precomposed U+00F1 on the left and `n` plus U+0303 on the right, "
    "which look the same but differ (no normalization, spec 7.1): left `[2, 3)`, right "
    "`[2, 4)`. L3 drops U+0323 from a stacked cluster: left `[0, 3)`, right `[0, 2)`. "
    "A correct result has one `replace` of L1-3/R1-3 holding exactly these six spans.",
)

# 033 -------------------------------------------------------------------------
case(
    "033-smart-prose",
    "txt",
    lf(
        "Harbour notes",
        "The colour of the harbour changes with the weather.",
        "We walked to the old lighthouse after lunch.",
        "Then we went home.",
    ),
    lf(
        "Harbour notes",
        "The color of the harbor changes with the weather.",
        "We walked to the new lighthouse after dinner.",
        "Then we went home.",
    ),
    opts(granularity="smart"),
    "Runs `granularity: \"smart\"` on prose with four word changes in one `replace` of "
    "L2-3/R2-3. `colour`/`color` and `harbour`/`harbor` are one-word swaps of at least "
    "4 graphemes that are similar enough for the sub-word pass (moves spec 3.3 step "
    "3), so only the `u` is marked: left spans `[8, 9)` and `[23, 24)` on L2, and no "
    "right span on R2. `old`/`new` is under 4 graphemes and `lunch`/`dinner` is not "
    "similar enough, so they keep whole-word spans: left `[17, 20)` and `[38, 43)` on "
    "L3, right `[17, 20)` and `[38, 44)` on R3. Offsets are UTF-8 bytes. Most text is "
    "equal, so the noise cutoff does not apply.",
)

# 034 -------------------------------------------------------------------------
case(
    "034-smart-noise",
    "txt",
    lf(
        "Daily log",
        "Invoice paid by bank transfer on Friday.",
        "Signed off.",
    ),
    lf(
        "Daily log",
        "Remember to water the tomatoes twice a day.",
        "Signed off.",
    ),
    opts(granularity="smart"),
    "Runs `granularity: \"smart\"` where line 2 is replaced by an unrelated sentence. "
    "No word is shared, so almost every non-whitespace grapheme lies in a span and the "
    "smart noise cutoff (moves spec 3.3 step 4, 25 %) applies. A correct result has one "
    "hunk: equal L1, `replace` L2/R2 with `inline: []`, equal L3. The cutoff is not a "
    "size limit, so there is no `refine-skipped-limit` warning and `warnings` is "
    "empty.",
)

# 035 -------------------------------------------------------------------------
_rows_left = [f"row {n:03d}: status pending" for n in range(1, 102)]
_rows_right = [f"row {n:03d}: status shipped" for n in range(1, 102)]
_footer = [f"note {n}: unchanged" for n in range(1, 9)]
case(
    "035-refine-skipped-limit",
    "txt",
    lf("# batch export", *_rows_left, *_footer, "total rows: 101", "# end"),
    lf("# batch export", *_rows_right, *_footer, "total rows: 102", "# end"),
    opts(granularity="smart"),
    "Runs `granularity: \"smart\"`. Lines 2-102 (101 lines) all change, giving one "
    "`replace` of 101 lines per side, which is above `REFINE_MAX_LINES` (100, moves "
    "spec 3.4). Eight equal lines later, line 111 changes `101` to `102`. A correct "
    "result has two hunks: h1 with equal L1, the 101-line `replace` with `inline: []`, "
    "and equal L103-105; h2 with equal L108-110, a one-line `replace` at L111 with "
    "whole-word spans `[12, 15)` on both sides (3 graphemes, so no sub-word pass), and "
    "equal L112. `warnings` is exactly one `refine-skipped-limit` with `side: null`, "
    "and the status stays `different`.",
)


# 036 -------------------------------------------------------------------------
case(
    "036-blank-lines-ignored",
    "txt",
    _blank_left,
    _blank_right,
    opts(context=1, blank_lines=True),
    "The inputs of case 026, run with `ignore.blankLines: true` (spec 7.3 step 3). Every "
    "change op holds only blank lines, so each becomes an `ignored` op, which is not a "
    "change op and never starts a hunk. A correct result is `identical` with `hunks: []`, "
    "`changeCount: 0`, `hunkCount: 0`, `linesAdded: 0`, `linesRemoved: 0` and "
    "`linesUnchanged: 8`. The three ignored lines (one left, two right) leave the "
    "similarity denominator (spec 6.3), so `similarity` is `1` = 2 x 8 / (8 + 8).",
)

# 037 -------------------------------------------------------------------------
case(
    "037-blank-lines-near-changes",
    "txt",
    lf(
        "# Shopping",
        "",
        "apples",
        "bread",
        "cheese",
        "",
        "dates",
        "eggs",
        "figs",
        "grapes",
        "honey",
        "ice cream",
        "",
        "",
        "jam",
    ),
    lf(
        "# Shopping",
        "apples",
        "bread",
        "butter",
        "cheese",
        "",
        "",
        "dates",
        "eggs",
        "figs",
        "grapes",
        "honey",
        "ice cream",
        "",
        "jam",
        "",
        "kale",
    ),
    opts(context=3, blank_lines=True),
    "Runs `ignore.blankLines: true` with `context: 3`. Two real changes (`butter` "
    "inserted after `bread`, and `\"\"` plus `kale` appended at the end) sit among three "
    "blank-only changes: the blank line after the heading is removed, a blank line is "
    "added after `cheese`, and two blank lines before `jam` become one. The appended "
    "insert holds a blank line and `kale`, so it stays an `insert` (a change op with any "
    "non-blank line is never split). Context counts equal lines only, and ignored ops "
    "are kept whole (spec 7.4). Which blank line of a pair is matched is the histogram "
    "choice. A correct result has two hunks. h1: equal L1, `ignored` L2 (`rightRange "
    "{start: 2, count: 0}`), equal L3-4, `insert` R4, equal L5, `ignored` R6 "
    "(`leftRange {start: 6, count: 0}`), equal L6-7 (three equal lines on each side of "
    "the insert). h2: equal L12-13, `ignored` L14 (`rightRange {start: 15, count: 0}`), "
    "equal L15, `insert` R16-17. The ten equal lines between the two inserts are more "
    "than 2 x 3, so the hunks stay apart. Stats: `changeCount: 2`, `linesAdded: 3`, "
    "`linesRemoved: 0`, `linesUnchanged: 13` and `similarity: 0.8966` = 2 x 13 / "
    "((15 - 2) + (17 - 1)).",
)

# 038 -------------------------------------------------------------------------
_uuid = r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
case(
    "038-regex-uuid-durations",
    "log",
    lf(
        "GET /api/orders 200 req=3f2b8c1e-9a4d-4e7b-8c21-5d6f7a8b9c0d in 12 ms",
        "GET /api/orders/17 200 req=a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d in 8 ms",
        "POST /api/orders 201 req=0f1e2d3c-4b5a-4968-8776-5a4b3c2d1e0f in 31 ms",
        "GET /health 200 in 1 ms",
        "GET /api/users/4 200 req=9e8d7c6b-5a49-4382-9170-6f5e4d3c2b1a in 5 ms",
        "DELETE /api/orders/17 204 req=7c6b5a49-3827-4165-9f4e-3d2c1b0a9f8e in 9 ms",
    ),
    lf(
        "GET /api/orders 200 req=5d4c3b2a-1f0e-4d9c-8b7a-6f5e4d3c2b1a in 15 ms",
        "GET /api/orders/17 404 req=b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e in 3 ms",
        "POST /api/orders 201 req=1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d in 28 ms",
        "GET /health 200 in 2 ms",
        "GET /api/users/4 200 req= in 6 ms",
        "DELETE /api/orders/17 204 req=8d7c6b5a-4938-4271-8e6f-5d4c3b2a1f0e in 11 ms",
    ),
    opts(regex=[_uuid, r"\d+ ms$"], context=1),
    "Two request logs in which every request id (a UUID) and every duration differs. "
    "`ignore.regex` holds two patterns: lowercase UUIDs, and a duration at the end of the "
    "line. Each match is replaced with its pattern's placeholder (spec 7.2.1), so lines 1, "
    "3, 4 and 6 compare equal. Two lines must still differ: L2 changes the status "
    "(`200` to `404`), and on the right L5 has an empty request id (`req= in 6 ms`). A "
    "placeholder keeps the presence of the masked value, so L5 differs; removing the "
    "matches instead would make the two L5 lines equal and hide the missing id. With "
    "`context: 1` a correct result has one hunk covering L1-6: equal L1, `replace` L2, "
    "equal L3-4, `replace` L5, equal L6; `linesUnchanged: 4`, `similarity: 0.6667`.",
)

# 039 -------------------------------------------------------------------------
_config_left = lf("[service]", "name = billing", "port = 8080", "workers = 4")
_config_right = lf("[service]", "name = billing", "port = 8081", "workers = 4")
case(
    "039-regex-invalid",
    "ini",
    _config_left,
    _config_right,
    opts(regex=[r"port = \d+", r"(?P<count>\d+"]),
    "`ignore.regex` holds a valid pattern and an invalid one (an unclosed group). The "
    "options object itself is well formed, so the API returns a document, not an "
    "error (spec 9). A correct result is `status: \"error\"` with exactly one "
    "`invalid-regex` error (`side: null`), `hunks: []`, `moves: []` and `stats: null`. "
    "Both file metas are filled in (`lineCount: 4` each) and both patterns are echoed "
    "in `options` in the given order.",
)

# 040 -------------------------------------------------------------------------
case(
    "040-regex-limit-exceeded",
    "ini",
    _config_left,
    _config_right,
    opts(regex=[r"\w{32}"]),
    "`ignore.regex` holds `\\w{32}`: 32 Unicode word characters in a row. It is a valid "
    "pattern, but its forward NFA has more than 50000 units (states plus transitions), "
    "the limit of spec 7.2.1, while `\\w{31}` is within it. The unit count does not depend "
    "on the platform, so a 32-bit WASM build must give the same result as a 64-bit "
    "native one. A correct result is `status: \"error\"` with exactly one "
    "`limit-exceeded` error (`side: null`), `hunks: []` and `stats: null`, with both "
    "file metas filled in and the pattern echoed in `options`.",
)

# 041 -------------------------------------------------------------------------
case(
    "041-regex-inline-spans",
    "log",
    lf(
        "2026-05-02T10:00:01Z backup started for volume data-01",
        "2026-05-02T10:04:17Z backup finished in 256 s with 14 files skipped",
        "2026-05-02T10:04:18Z upload queued to bucket nightly",
    ),
    lf(
        "2026-05-03T10:00:02Z backup started for volume data-02",
        "2026-05-03T10:05:44Z backup finished in 342 s with 9 files skipped",
        "2026-05-03T10:05:45Z upload queued to bucket nightly",
    ),
    opts(
        granularity="word",
        regex=[r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", r"in \d+ s"],
    ),
    "Two backup logs with new timestamps and durations, masked by `ignore.regex`. L3 "
    "differs only in masked text, so it is equal; L1 and L2 form one `replace` op. "
    "Inline refinement applies the same masks (moves spec 3.2): each match is one token "
    "equal to any match of the same pattern, so the timestamps and `in 256 s` / "
    "`in 342 s` get no span. The only word spans are the volume number (`01` / `02`) "
    "and the skipped count (`14` / `9`).",
)


def write_case(entry: dict) -> None:
    directory = ROOT / entry["slug"]
    directory.mkdir(parents=True, exist_ok=True)
    for side in ("left", "right"):
        data = entry[side]
        if len(data) > entry["max_bytes"]:
            raise SystemExit(f"{entry['slug']}: {side} is {len(data)} bytes, over the limit")
        (directory / f"{side}.{entry['ext']}").write_bytes(data)
    options_text = json.dumps(entry["options"], indent=2, ensure_ascii=False) + "\n"
    (directory / "options.json").write_text(options_text, encoding="utf-8")
    about = f"# {entry['slug']}\n\n{entry['about']}\n"
    (directory / "case.md").write_text(about, encoding="utf-8")


def self_check() -> None:
    slugs = [entry["slug"] for entry in CASES]
    if len(slugs) != 41 or len(set(slugs)) != 41:
        raise SystemExit(f"expected 41 unique cases, found {len(slugs)}")
    for number, slug in enumerate(slugs, start=1):
        if not slug.startswith(f"{number:03d}-"):
            raise SystemExit(f"case {slug} is out of order")
    by_slug = {entry["slug"]: entry for entry in CASES}
    assert by_slug["002-both-empty"]["left"] == b"" == by_slug["002-both-empty"]["right"]
    assert by_slug["008-no-final-newline"]["right"][-1:] != b"\n"
    assert b"\r\n" in by_slug["009-crlf-vs-lf"]["right"]
    assert b"\r" not in by_slug["009-crlf-vs-lf"]["left"]
    assert by_slug["010-utf8-bom"]["right"].startswith(BOM)
    assert not by_slug["010-utf8-bom"]["left"].startswith(BOM)
    assert by_slug["024-lone-cr"]["left"].count(b"\r") == 6
    assert b"\n" not in by_slug["024-lone-cr"]["left"] + by_slug["024-lone-cr"]["right"]
    assert b"\n" not in by_slug["025-lone-cr-vs-lf"]["left"]
    assert b"\r" not in by_slug["025-lone-cr-vs-lf"]["right"]
    assert by_slug["027-only-newline"]["left"] == b"\n"
    assert by_slug["022-utf16-bom"]["right"].startswith(b"\xff\xfe")
    for entry in CASES:
        for side in ("left", "right"):
            data = entry[side]
            if entry["valid_utf8"] or side == "left":
                data.decode("utf-8")  # every input except the invalid sides must be UTF-8
            else:
                try:
                    data.decode("utf-8")
                except UnicodeDecodeError:
                    continue
                raise SystemExit(f"{entry['slug']}: {side} was meant to be invalid UTF-8")


def main() -> int:
    self_check()
    for entry in CASES:
        write_case(entry)
    print(f"wrote {len(CASES)} text cases under {ROOT.relative_to(ROOT.parent.parent.parent)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
