#!/usr/bin/env python3
"""Check the text cases' expected.json against the inputs, independently of the engine.

expected.json is written by the engine (``diffyman-conformance --bless``). This
script re-derives what it can from the input bytes with its own code and fails
on any disagreement:

- file meta: size, sha256, bom, encoding, lineTerminator, finalNewline,
  lineCount (diff-document.md sections 6.1, 7.1, 7.2 and 8);
- per-side warnings and the status rules (sections 6 and 10), and the
  `ignore.regex` errors listed in REGEX_ERRORS;
- hunk shape: ids, ops tiling the hunk ranges, op kinds and counts, the
  context rule (section 7.4) and the stats formulas (section 6.3);
- `ignored` ops (section 7.3 step 3): only under `ignore.blankLines`, blank
  lines only, counted as neither changed nor unchanged; outside the hunks,
  the non-blank lines of both sides must pair up;
- every line inside an `equal` op compares equal under the case's whitespace
  mode and regex masks (section 7.2; skipped for `ignore.case: true`, which
  needs Unicode simple case folding). Masks use Python's `re`, which agrees
  with the Rust dialect for the corpus patterns;
- inline spans: side/op pairing, order, bounds and unit boundaries for the
  case's positionEncoding (section 6.4), and, for the cases listed in SPANS,
  the exact text each span covers.

Usage (from the repository root):

    uv run --python 3.12 python tests/corpus/tools/check_text_expected.py

All content in this file is original and written for the diffyman corpus.
"""

from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "text"
BOM = b"\xef\xbb\xbf"
WHITESPACE = "\t\x0b\x0c "

# Exact span texts for the granularity cases: (side, line, covered text).
SPANS: dict[str, list[tuple[str, int, str]]] = {
    "031-utf16-word-cjk-emoji": [
        ("left", 1, "\U0001F363"),
        ("left", 2, "\U0001F44D\U0001F3FD"),
        ("left", 3, "東京"),
        ("left", 3, "\U0001F5FC"),
        ("right", 1, "\U0001F35C"),
        ("right", 2, "\U0001F44D\U0001F3FF"),
        ("right", 3, "大阪"),
        ("right", 3, "\U0001F3EF"),
    ],
    "032-codepoint-char-combining": [
        ("left", 1, "é"),
        ("left", 2, "ñ"),
        ("left", 3, "ẋ̣"),
        ("right", 1, "è"),
        ("right", 2, "ñ"),
        ("right", 3, "ẋ"),
    ],
    "033-smart-prose": [
        ("left", 2, "u"),
        ("left", 2, "u"),
        ("left", 3, "old"),
        ("left", 3, "lunch"),
        ("right", 3, "new"),
        ("right", 3, "dinner"),
    ],
    "034-smart-noise": [],
    "035-refine-skipped-limit": [("left", 111, "101"), ("right", 111, "102")],
    # Masked timestamps and durations never get a span (moves spec 3.2).
    "041-regex-inline-spans": [
        ("left", 1, "01"),
        ("left", 2, "14"),
        ("right", 1, "02"),
        ("right", 2, "9"),
    ],
    # UAX #29 keeps letters joined by a full stop in one word segment.
    "045-js-rename-word-utf16": [
        ("left", 2, "usr"),
        ("left", 3, "usr.name"),
        ("left", 4, "usr.locale"),
        ("right", 2, "user"),
        ("right", 3, "user.name"),
        ("right", 4, "user.locale"),
    ],
    "047-go-smart-error-wrap": [
        ("right", 4, 'fmt.Errorf("read %s: %w", path, '),
        ("right", 4, ")"),
        ("right", 8, 'fmt.Errorf("parse %s: %w", path, '),
        ("right", 8, ")"),
    ],
    "048-rust-char-utf8": [
        ("left", 3, "z"),
        ("right", 3, "f"),
        ("left", 5, "9"),
        ("left", 6, "ö"),
        ("right", 5, "7"),
        ("right", 6, "ő"),
    ],
    "054-markdown-smart-codepoint": [
        ("left", 3, "8"),
        ("left", 3, "00"),
        ("left", 4, "û"),
        ("right", 3, "7"),
        ("right", 3, "30"),
        ("right", 4, "u"),
    ],
    "055-html-word-codepoint": [
        ("left", 2, "en"),
        ("right", 2, "sv"),
        ("left", 6, "length"),
        ("right", 5, " warning"),
        ("right", 6, "size"),
    ],
    # Masked commit ids never get a span.
    "065-regex-commit-ids-word": [
        ("left", 2, "folder"),
        ("right", 2, "directory"),
        ("left", 4, "lazily"),
        ("right", 4, "on demand"),
    ],
    "066-char-utf16-astral": [
        ("left", 1, "\U0001D465"),
        ("left", 1, "\U0001D465"),
        ("left", 2, "太"),
        ("left", 3, "\U0001F642"),
        ("right", 1, "\U0001D466"),
        ("right", 1, "\U0001D466"),
        ("right", 2, "次"),
        ("right", 3, "\U0001F643"),
    ],
    "067-smart-utf16-typescript": [
        ("left", 3, "document"),
        ("left", 4, "\U0001F4F4 You are offline"),
        ("right", 3, "file"),
        ("right", 4, "\U0001F4F6 Reconnecting…"),
    ],
    "068-smart-codepoint-french": [
        ("left", 1, "è"),
        ("left", 2, "écrit"),
        ("right", 1, "e"),
        ("right", 2, "rédigé"),
    ],
    "069-word-codepoint-cyrillic-greek": [
        ("left", 1, "Москва"),
        ("left", 1, "столица"),
        ("left", 2, "πρωτεύουσα"),
        ("right", 1, "Санкт-Петербург"),
        ("right", 1, " город в"),
        ("right", 2, " μεγαλύτερη πόλη"),
    ],
    "070-char-utf8-versions": [
        ("left", 2, "19"),
        ("left", 3, "1"),
        ("left", 3, "1"),
        ("right", 2, "28"),
        ("right", 3, "2"),
        ("right", 3, "2"),
    ],
    # The smart cases of moves spec section 3.5.
    "074-smart-identifier": [("left", 1, "N"), ("right", 1, "n")],
    "075-smart-short-word": [("left", 1, "bar"), ("right", 1, "baz")],
    "076-smart-two-words": [("left", 1, "red apple"), ("right", 1, "green pear")],
    "077-smart-cjk": [("left", 1, "猫"), ("right", 1, "狗")],
    "078-smart-thai": [("left", 1, "แ"), ("left", 1, "ว"), ("right", 1, "ห"), ("right", 1, "า")],
    "079-smart-emoji-zwj": [
        ("left", 1, "\U0001F469‍\U0001F4BB"),
        ("right", 1, "\U0001F468‍\U0001F4BB"),
    ],
    "080-smart-insert-only": [],
}

# Cases whose ignore.regex must fail, with the expected `side: null` codes. The
# NFA size limit is not recomputed here; an invalid pattern must also fail to
# compile in Python.
REGEX_ERRORS: dict[str, list[str]] = {
    "039-regex-invalid": ["invalid-regex"],
    "040-regex-limit-exceeded": ["limit-exceeded"],
}


def split_lines(text: str) -> list[tuple[str, str]]:
    """(content, terminator) pairs; LF, CRLF and a lone CR end a line."""
    lines = []
    start = i = 0
    while i < len(text):
        ch = text[i]
        if ch == "\n":
            lines.append((text[start:i], "lf"))
            i += 1
            start = i
        elif ch == "\r":
            if text[i + 1 : i + 2] == "\n":
                lines.append((text[start:i], "crlf"))
                i += 2
            else:
                lines.append((text[start:i], "cr"))
                i += 1
            start = i
        else:
            i += 1
    if start < len(text):
        lines.append((text[start:], "none"))
    return lines


def is_blank(content: str) -> bool:
    return all(ch in WHITESPACE for ch in content)


def masked(content: str, patterns: list[re.Pattern[str]]) -> list[str | int]:
    """Text pieces and placeholder indexes (section 7.2.1): non-empty matches,
    earlier patterns win overlaps."""
    kept: list[tuple[int, int, int]] = []
    for index, pattern in enumerate(patterns):
        for m in pattern.finditer(content):
            if m.start() == m.end():
                continue
            if any(s < m.end() and m.start() < e for s, e, _ in kept):
                continue
            kept.append((m.start(), m.end(), index))
    kept.sort()
    out: list[str | int] = []
    pos = 0
    for start, end, index in kept:
        out.extend([content[pos:start], index])
        pos = end
    out.append(content[pos:])
    return out


def collapse(text: str) -> str:
    out, in_run = [], False
    for ch in text:
        if ch in WHITESPACE:
            if not in_run:
                out.append(" ")
            in_run = True
        else:
            out.append(ch)
            in_run = False
    return "".join(out)


def key(content: str, terminator: str, mode: str, patterns: list[re.Pattern[str]] | None = None) -> tuple:
    pieces = masked(content, patterns or [])
    out: list[str | int] = []
    for i, piece in enumerate(pieces):
        if isinstance(piece, int):
            out.append(piece)
            continue
        if i == len(pieces) - 1 and mode in ("trailing", "change"):
            piece = piece.rstrip(WHITESPACE)
        if mode == "change":
            piece = collapse(piece)
        elif mode == "all":
            piece = "".join(ch for ch in piece if ch not in WHITESPACE)
        elif mode not in ("none", "trailing"):
            raise ValueError(mode)
        out.append(piece)
    if mode == "none":
        out.append(("terminator", terminator))
    return tuple(out)


def units(text: str, encoding: str) -> list[int]:
    """Cumulative offsets at every code point boundary, in the given unit."""
    out, n = [0], 0
    for ch in text:
        if encoding == "utf-8":
            n += len(ch.encode("utf-8"))
        elif encoding == "utf-16":
            n += 2 if ord(ch) > 0xFFFF else 1
        else:
            n += 1
        out.append(n)
    return out


def slice_units(text: str, start: int, end: int, encoding: str) -> str:
    bounds = units(text, encoding)
    if start not in bounds or end not in bounds:
        raise AssertionError(f"offsets {start}..{end} split a code point in {text!r}")
    return text[bounds.index(start) : bounds.index(end)]


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


def decode(raw: bytes) -> str | None:
    body = raw[3:] if raw.startswith(BOM) else raw
    try:
        return body.decode("utf-8", errors="strict")
    except UnicodeDecodeError:
        return None


# Media types of the text extensions (diff-document.md section 16.1, the text
# entries of the detect.sniff@1 table). Any other extension of a text input
# (.py, .go, .rs, .ts, .sql, .sh, ...) gives text/plain; no text/ input starts
# like XML or HTML without one of these extensions.
TEXT_MEDIA_TYPES = {
    "css": "text/css",
    "csv": "text/csv",
    "htm": "text/html",
    "html": "text/html",
    "js": "text/javascript",
    "json": "application/json",
    "markdown": "text/markdown",
    "md": "text/markdown",
    "mjs": "text/javascript",
    "tsv": "text/tab-separated-values",
    "xml": "application/xml",
    "yaml": "application/yaml",
    "yml": "application/yaml",
}


def is_utf16_text(raw: bytes) -> bool:
    if raw[:2] not in (b"\xff\xfe", b"\xfe\xff") or len(raw) % 2:
        return False
    try:
        text = raw[2:].decode("utf-16-le" if raw[:2] == b"\xff\xfe" else "utf-16-be")
    except UnicodeDecodeError:
        return False
    return "\x00" not in text[:4000]


def media_type(ext: str, raw: bytes, text: str | None) -> str:
    """The detected media type of a text/ input (no case here has magic bytes)."""
    if text is not None or is_utf16_text(raw):
        return TEXT_MEDIA_TYPES.get(ext.lower(), "text/plain")
    return "application/octet-stream"


def check_meta(c: Checker, side: str, raw: bytes, meta: dict, text: str | None, lines, ext: str) -> None:
    c.eq(f"{side}.size", meta["size"], len(raw))
    c.eq(f"{side}.sha256", meta["sha256"], hashlib.sha256(raw).hexdigest())
    c.eq(f"{side}.bom", meta["bom"], raw.startswith(BOM))
    c.eq(f"{side}.mediaType", meta["mediaType"], media_type(ext, raw, text))
    if text is None:
        for field in ("encoding", "lineTerminator", "finalNewline", "lineCount"):
            c.eq(f"{side}.{field}", meta[field], None)
        return
    styles = sorted({t for _, t in lines if t != "none"})
    style = "none" if not styles else styles[0] if len(styles) == 1 else "mixed"
    c.eq(f"{side}.encoding", meta["encoding"], "utf-8")
    c.eq(f"{side}.lineTerminator", meta["lineTerminator"], style)
    c.eq(f"{side}.finalNewline", meta["finalNewline"], bool(lines) and lines[-1][1] != "none")
    c.eq(f"{side}.lineCount", meta["lineCount"], len(lines))


def expected_side_warnings(side: str, raw: bytes, text: str | None, lines) -> list[str]:
    if text is None:
        return ["invalid-utf8"]
    codes = []
    if raw.startswith(BOM):
        codes.append("bom-stripped")
    if len({t for _, t in lines if t != "none"}) > 1:
        codes.append("mixed-line-endings")
    if lines and lines[-1][1] == "none":
        codes.append("no-newline-at-eof")
    return sorted(codes)


def check_hunks(c: Checker, doc: dict, left, right, options: dict) -> None:
    context = options["context"]
    mode = options["ignore"]["whitespace"]
    patterns = [re.compile(p) for p in options["ignore"]["regex"]]
    blank_lines = options["ignore"]["blankLines"]
    encoding = options["positionEncoding"]
    granularity = options["granularity"]
    changes = equal_lines = 0
    added = removed = 0
    ignored_left = ignored_right = 0
    # Line ranges (0-based, half-open) covered by hunks, per side.
    covered: list[tuple[int, int, int, int]] = []
    prev_end = (1, 1)
    spans_seen: list[tuple[str, int, str]] = []
    for index, hunk in enumerate(doc["hunks"]):
        c.eq(f"hunk {index} id", hunk["id"], f"h{index + 1}")
        ops = hunk["ops"]
        c.true(f"{hunk['id']} has no ops", bool(ops))
        if not ops:
            continue
        pos = (ops[0]["leftRange"]["start"], ops[0]["rightRange"]["start"])
        c.true(f"{hunk['id']} starts before the previous hunk ended", pos >= prev_end)
        c.eq(f"{hunk['id']}.left.start", hunk["left"]["start"], pos[0])
        c.eq(f"{hunk['id']}.right.start", hunk["right"]["start"], pos[1])
        c.eq(f"{hunk['id']}.left.count", hunk["left"]["count"], sum(o["leftRange"]["count"] for o in ops))
        c.eq(f"{hunk['id']}.right.count", hunk["right"]["count"], sum(o["rightRange"]["count"] for o in ops))
        kinds = [o["op"] for o in ops]
        for a, b in zip(kinds, kinds[1:]):
            c.true(f"{hunk['id']}: two non-equal ops are adjacent", a == "equal" or b == "equal")
        change_at = [k for k, kind in enumerate(kinds) if kind in ("insert", "delete", "replace")]
        c.true(f"{hunk['id']} has no change op", bool(change_at))
        if change_at:
            # Context counts equal lines only (section 7.4).
            def equal_in(run: list[dict]) -> int:
                return sum(o["leftRange"]["count"] for o in run if o["op"] == "equal")

            c.true(f"{hunk['id']}: leading context over {context}", equal_in(ops[: change_at[0]]) <= context)
            c.true(f"{hunk['id']}: trailing context over {context}", equal_in(ops[change_at[-1] + 1 :]) <= context)
            for a, b in zip(change_at, change_at[1:]):
                c.true(f"{hunk['id']}: more than 2*context equal lines between changes", equal_in(ops[a + 1 : b]) <= 2 * context)
            if kinds[0] == "ignored":
                c.true(f"{hunk['id']}: starts with an ignored op away from the file start", pos == (1, 1))
            if kinds[-1] == "ignored":
                end = (
                    ops[-1]["leftRange"]["start"] + ops[-1]["leftRange"]["count"],
                    ops[-1]["rightRange"]["start"] + ops[-1]["rightRange"]["count"],
                )
                c.true(f"{hunk['id']}: ends with an ignored op away from the file end", end == (len(left) + 1, len(right) + 1))
        for k, op in enumerate(ops):
            lr, rr = op["leftRange"], op["rightRange"]
            where = f"{hunk['id']} op {k}"
            c.eq(f"{where} left start", lr["start"], pos[0])
            c.eq(f"{where} right start", rr["start"], pos[1])
            kind = op["op"]
            shape = {
                "equal": lr["count"] == rr["count"] > 0,
                "insert": lr["count"] == 0 < rr["count"],
                "delete": rr["count"] == 0 < lr["count"],
                "replace": lr["count"] > 0 and rr["count"] > 0,
                "ignored": lr["count"] + rr["count"] > 0,
            }
            c.true(f"{where}: counts do not fit `{kind}`", shape.get(kind, False))
            if kind == "equal":
                equal_lines += lr["count"]
                if not options["ignore"]["case"]:
                    for j in range(lr["count"]):
                        a = left[lr["start"] - 1 + j]
                        b = right[rr["start"] - 1 + j]
                        c.true(
                            f"{where}: L{lr['start'] + j} and R{rr['start'] + j} are not equal under {mode}",
                            key(*a, mode, patterns) == key(*b, mode, patterns),
                        )
            elif kind == "ignored":
                c.true(f"{where}: `ignored` op without ignore.blankLines", blank_lines)
                lines_here = left[lr["start"] - 1 : lr["start"] - 1 + lr["count"]]
                lines_here += right[rr["start"] - 1 : rr["start"] - 1 + rr["count"]]
                c.true(f"{where}: `ignored` op holds a non-blank line", all(is_blank(t) for t, _ in lines_here))
                ignored_left += lr["count"]
                ignored_right += rr["count"]
            else:
                if blank_lines:
                    lines_here = left[lr["start"] - 1 : lr["start"] - 1 + lr["count"]]
                    lines_here += right[rr["start"] - 1 : rr["start"] - 1 + rr["count"]]
                    c.true(f"{where}: blank-only change not ignored", not all(is_blank(t) for t, _ in lines_here))
                changes += 1
                added += rr["count"]
                removed += lr["count"]
            spans = op["inline"]
            if kind != "replace" or granularity == "line":
                c.true(f"{where}: unexpected inline spans", not spans)
            order = [(0 if s["side"] == "left" else 1, s["line"], s["start"]) for s in spans]
            c.true(f"{where}: spans are not sorted", order == sorted(order))
            last_end: dict[tuple[str, int], int] = {}
            for s in spans:
                side_lines, rng = (left, lr) if s["side"] == "left" else (right, rr)
                c.eq(f"{where} span op", s["op"], "delete" if s["side"] == "left" else "insert")
                inside = rng["start"] <= s["line"] < rng["start"] + rng["count"]
                c.true(f"{where}: span line {s['line']} is outside the op", inside)
                if not inside:
                    continue
                content = side_lines[s["line"] - 1][0]
                c.true(f"{where}: empty or inverted span", 0 <= s["start"] < s["end"] <= units(content, encoding)[-1])
                c.true(
                    f"{where}: overlapping spans on {s['side']} line {s['line']}",
                    s["start"] >= last_end.get((s["side"], s["line"]), 0),
                )
                last_end[(s["side"], s["line"])] = s["end"]
                try:
                    spans_seen.append((s["side"], s["line"], slice_units(content, s["start"], s["end"], encoding)))
                except AssertionError as err:
                    c.problems.append(f"{where}: {err}")
            pos = (pos[0] + lr["count"], pos[1] + rr["count"])
        covered.append((hunk["left"]["start"] - 1, pos[0] - 1, hunk["right"]["start"] - 1, pos[1] - 1))
        prev_end = pos
    stats = doc["stats"]
    c.eq("stats.linesLeft", stats["linesLeft"], len(left))
    c.eq("stats.linesRight", stats["linesRight"], len(right))
    c.eq("stats.hunkCount", stats["hunkCount"], len(doc["hunks"]))
    c.eq("stats.changeCount", stats["changeCount"], changes)
    c.eq("stats.linesAdded", stats["linesAdded"], added)
    c.eq("stats.linesRemoved", stats["linesRemoved"], removed)
    # Outside the hunks every line is equal, or (under blankLines) blank in an
    # ignored op. So the non-blank lines of each gap pair up in order, and the
    # ignored lines outside hunks are at most the blank lines there.
    gaps, (l0, r0) = [], (0, 0)
    for ls, le, rs, re_ in covered:
        gaps.append((l0, ls, r0, rs))
        l0, r0 = le, re_
    gaps.append((l0, len(left), r0, len(right)))
    blank_gap_left = blank_gap_right = 0
    # Bounds on the equal lines outside hunks: at least the paired non-blank
    # lines, at most the shorter side of each gap. How blank runs outside the
    # hunks align is the algorithm's choice and is not re-derived here.
    gap_equal_min = gap_equal_max = 0
    for ls, le, rs, re_ in gaps:
        gl, gr = left[ls:le], right[rs:re_]
        gap_equal_max += min(len(gl), len(gr))
        if blank_lines:
            blank_gap_left += sum(is_blank(t) for t, _ in gl)
            blank_gap_right += sum(is_blank(t) for t, _ in gr)
            gl = [line for line in gl if not is_blank(line[0])]
            gr = [line for line in gr if not is_blank(line[0])]
        same = len(gl) == len(gr) and (
            options["ignore"]["case"] or all(key(*a, mode, patterns) == key(*b, mode, patterns) for a, b in zip(gl, gr))
        )
        c.true(f"lines L{ls + 1}-{le} / R{rs + 1}-{re_} outside the hunks do not pair up", same)
        gap_equal_min += min(len(gl), len(gr))
    unchanged = stats["linesUnchanged"]
    c.true(
        "stats.linesUnchanged is outside the bounds the inputs allow",
        equal_lines + gap_equal_min <= unchanged <= equal_lines + gap_equal_max,
    )
    il = len(left) - removed - unchanged
    ir = len(right) - added - unchanged
    if blank_lines:
        c.true("ignored left lines fit", ignored_left <= il <= ignored_left + blank_gap_left)
        c.true("ignored right lines fit", ignored_right <= ir <= ignored_right + blank_gap_right)
    else:
        c.eq("stats.linesUnchanged", unchanged, len(left) - removed)
        c.eq("lines outside changes agree", len(right) - added, unchanged)
    total = len(left) - il + len(right) - ir
    bp = 10000 if total == 0 else (40000 * unchanged + total) // (2 * total)
    c.eq("stats.similarity", stats["similarity"], bp / 10000 if bp % 10000 else bp // 10000)
    if c.name in SPANS:
        c.eq("span texts (from the offsets vs the SPANS table)", spans_seen, SPANS[c.name])


def check_case(directory: Path) -> list[str]:
    c = Checker(directory.name)
    doc = json.loads((directory / "expected.json").read_text(encoding="utf-8"))
    options = json.loads((directory / "options.json").read_text(encoding="utf-8"))
    paths = {side: next(directory.glob(f"{side}.*")) for side in ("left", "right")}
    raws = {side: path.read_bytes() for side, path in paths.items()}
    texts = {side: decode(raw) for side, raw in raws.items()}
    lines = {side: split_lines(t) if t is not None else None for side, t in texts.items()}
    for side in ("left", "right"):
        check_meta(c, side, raws[side], doc[side], texts[side], lines[side], paths[side].suffix[1:])
    c.eq("options", doc["options"], options)
    c.eq("positionEncoding", doc["positionEncoding"], options["positionEncoding"])
    deferred = bool(options["detectMoves"])  # covered by the moved/ cases, not here
    c.eq("kind", doc["kind"], "text")
    c.eq("bytes", doc["bytes"], [])
    if doc["stats"] is not None:
        c.eq("stats.bytes", doc["stats"]["bytes"], None)
    if None in texts.values():
        c.eq("an input that is not UTF-8 needs the text adapter", options["adapter"], "text")
    regex_errors = REGEX_ERRORS.get(directory.name, [])
    if "invalid-regex" in regex_errors:
        failures = 0
        for pattern in options["ignore"]["regex"]:
            try:
                re.compile(pattern)
            except re.error:
                failures += 1
        c.true("an invalid-regex case needs a pattern that fails to compile", failures > 0)
    have_null = [w["code"] for w in doc["warnings"] if w["side"] is None]
    if regex_errors:
        c.eq("regex errors", have_null, regex_errors)
    else:
        c.true("unexpected regex error", not {"invalid-regex", "limit-exceeded"} & set(have_null))
    want = [(code, "left") for code in expected_side_warnings("left", raws["left"], texts["left"], lines["left"])]
    want += [(code, "right") for code in expected_side_warnings("right", raws["right"], texts["right"], lines["right"])]
    have_side = [(w["code"], w["side"]) for w in doc["warnings"] if w["side"] is not None]
    c.eq("per-side warnings", have_side, want)
    errors = any(w["severity"] == "error" for w in doc["warnings"])
    c.eq("error status", doc["status"] == "error", errors)
    if None in texts.values() or deferred or regex_errors:
        c.true("an error document must have status error", doc["status"] == "error")
        c.eq("hunks", doc["hunks"], [])
        c.eq("stats", doc["stats"], None)
    if doc["status"] != "error":
        check_hunks(c, doc, lines["left"], lines["right"], options)
        c.eq("identical status", doc["status"] == "identical", doc["stats"]["changeCount"] == 0)
        skipped = any(w["code"] == "refine-skipped-limit" for w in doc["warnings"])
        if skipped:
            c.true("refine-skipped-limit needs granularity other than line", options["granularity"] != "line")
    c.eq("moves", doc["moves"], [])
    return c.problems


def main() -> int:
    root = Path(sys.argv[2]) if len(sys.argv) == 3 and sys.argv[1] == "--root" else ROOT
    failed = 0
    cases = sorted(p for p in root.iterdir() if (p / "expected.json").is_file())
    for directory in cases:
        problems = check_case(directory)
        print(f"{'ok  ' if not problems else 'FAIL'} {directory.name}")
        for problem in problems:
            print(f"     {problem}")
        failed += bool(problems)
    missing = [name for name in SPANS if not (root / name / "expected.json").is_file()]
    for name in missing:
        print(f"FAIL {name}: listed in SPANS but has no expected.json")
    print(f"\n{len(cases)} text cases checked, {failed} with problems")
    return 1 if failed or missing else 0


if __name__ == "__main__":
    sys.exit(main())
