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
MAX_FILE_BYTES = 20 * 1024

BOM = b"\xef\xbb\xbf"


def opts(
    *,
    algorithm: str = "histogram",
    whitespace: str = "none",
    case: bool = False,
    regex: list[str] | None = None,
    context: int = 3,
    position: str = "utf-8",
) -> dict:
    """Return a fully spelled-out options object (diff-document.md section 9).

    Every key is written even when it equals the default, so a case keeps its
    meaning if a default ever changes. granularity stays "line" and
    detectMoves stays false: other values are provisional or deferred in
    schema 0.1.0.
    """
    return {
        "algorithm": algorithm,
        "granularity": "line",
        "ignore": {
            "whitespace": whitespace,
            "case": case,
            "blankLines": False,
            "regex": list(regex or []),
        },
        "context": context,
        "positionEncoding": position,
        "detectMoves": False,
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


def case(slug: str, ext: str, left: bytes, right: bytes, options: dict, about: str) -> None:
    CASES.append(
        {"slug": slug, "ext": ext, "left": left, "right": right, "options": options, "about": about}
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
    "the leading ISO 8601 timestamp. Non-empty `ignore.regex` is deferred to schema "
    "0.2.0 (spec section 9), so the correct 0.1.0 result is `status: \"error\"` with one "
    "`unsupported-option` error (`side: null`), `hunks: []`, `stats: null`, both file "
    "metas filled in, and the regex echoed in `options`. Once masking ships, the "
    "expected result becomes `replace` ops for the pid lines (L1-2), the latency line "
    "(L4) and the batch 3 line (L7) only.",
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


def write_case(entry: dict) -> None:
    directory = ROOT / entry["slug"]
    directory.mkdir(parents=True, exist_ok=True)
    for side in ("left", "right"):
        data = entry[side]
        if len(data) > MAX_FILE_BYTES:
            raise SystemExit(f"{entry['slug']}: {side} is {len(data)} bytes, over the limit")
        (directory / f"{side}.{entry['ext']}").write_bytes(data)
    options_text = json.dumps(entry["options"], indent=2, ensure_ascii=False) + "\n"
    (directory / "options.json").write_text(options_text, encoding="utf-8")
    about = f"# {entry['slug']}\n\n{entry['about']}\n"
    (directory / "case.md").write_text(about, encoding="utf-8")


def self_check() -> None:
    slugs = [entry["slug"] for entry in CASES]
    if len(slugs) != 20 or len(set(slugs)) != 20:
        raise SystemExit(f"expected 20 unique cases, found {len(slugs)}")
    by_slug = {entry["slug"]: entry for entry in CASES}
    assert by_slug["002-both-empty"]["left"] == b"" == by_slug["002-both-empty"]["right"]
    assert by_slug["008-no-final-newline"]["right"][-1:] != b"\n"
    assert b"\r\n" in by_slug["009-crlf-vs-lf"]["right"]
    assert b"\r" not in by_slug["009-crlf-vs-lf"]["left"]
    assert by_slug["010-utf8-bom"]["right"].startswith(BOM)
    assert not by_slug["010-utf8-bom"]["left"].startswith(BOM)
    for entry in CASES:
        for side in ("left", "right"):
            entry[side].decode("utf-8")  # every input must be valid UTF-8


def main() -> int:
    self_check()
    for entry in CASES:
        write_case(entry)
    print(f"wrote {len(CASES)} text cases under {ROOT.relative_to(ROOT.parent.parent.parent)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
