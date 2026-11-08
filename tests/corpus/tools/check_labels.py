#!/usr/bin/env python3
"""Check the moved-block labels in tests/corpus/moved/ against their files.

Run from the repository root:

    uv run --python 3.12 python tests/corpus/tools/check_labels.py

The script exits non-zero if any check fails. It checks, per case:

1. labels.json has exactly the shape of text-granularity-and-moves.md 5.2
   (keys, types, ranges inside the files, sort order, no overlap per side).
2. The input files follow the corpus rules: valid UTF-8, a final newline,
   no lone CR, and indentation made of spaces in steps of two. Lines are
   numbered by diff-document.md 7.2 (LF, CRLF and a lone CR end a line); lone
   CRs are still rejected because the git stand-in diff below treats a CR as
   content and would number lines differently.
3. Every label meets the minimum move size of section 4.2 on both sides.
4. A "pure" label pairs its lines one to one with equal move keys (section
   4.4 with ignore.whitespace "none": the case's ignore.regex masks applied,
   leading whitespace removed, the rest exact) and the same indentation delta
   on every non-blank line. An "edited" label reaches the block similarity
   and the word similarity of section 4.5 and is not secretly pure.
5. Line-diff consistency: using git's histogram diff (with its default slider
   compaction and indent heuristic, which text.histogram@1 follows, spec
   diff-document.md 7.3) as an independent stand-in for text.histogram@1,
   every labeled left line is
   deleted, every labeled right line is inserted, and the two sides are in
   different change ops (rule 4.1.4). The stand-in diffs the masked lines
   with LF terminators: rule 4.1.4 applies to the sub-ops of a replace op
   aligned by move key, which ignores terminators, so a whole-file LF to
   CRLF conversion behaves like a diff of the converted lines.
6. A small independent implementation of pure move detection (section 4.3)
   run on that diff must find every pure label (Jaccard >= 0.5 on both
   sides), must find nothing in negative cases, and must report nothing that
   no label explains.

It also validates every options.json in the corpus against the option shape
of diff-document.md section 9. The stand-in diff is only a sanity check: the
engine's own output is reviewed separately when expected.json is generated.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

CORPUS = Path(__file__).resolve().parent.parent
MOVED = CORPUS / "moved"
TEXT = CORPUS / "text"

TRIVIAL_LINE_MAX_ALNUM = 2
MOVE_MIN_ALNUM = 20
MOVE_MIN_NONTRIVIAL_LINES = 2
MOVE_SINGLE_LINE_MIN_ALNUM = 40
MOVE_EDITED_MIN_SIM_BP = 8000
MOVE_LINE_PAIR_MIN_BP = 6000
MOVE_EDITED_MIN_WORD_SIM_BP = 6000
MATCH_MIN_JACCARD = 0.5
# A regex placeholder in the masked text: one private-use character per pattern.
PLACEHOLDER_BASE = 0xF0000

OPTION_ENUMS = {
    "algorithm": {"histogram", "myers", "patience"},
    "granularity": {"line", "word", "char", "smart"},
    "positionEncoding": {"utf-8", "utf-16", "codepoint"},
    "adapter": {"auto", "text"},
}
WHITESPACE_MODES = {"none", "trailing", "change", "all"}


class CaseErrors:
    def __init__(self, name: str) -> None:
        self.name = name
        self.errors: list[str] = []
        self.info: list[str] = []

    def error(self, message: str) -> None:
        self.errors.append(message)


# --- text helpers -----------------------------------------------------------


LINE_END = re.compile(r"\r\n|\r|\n")
WHITESPACE = "\t\x0b\x0c "


def split_lines(text: str) -> list[str]:
    """Split into line contents by diff-document.md 7.2 (moves spec section 1).

    LF, CRLF and a lone CR each end a line; CR followed by LF is one CRLF
    terminator, and a terminator at the end of the input starts no new line.
    """
    if text == "":
        return []
    parts = LINE_END.split(text)
    if parts[-1] == "":
        parts.pop()
    return parts


def is_ws(ch: str) -> bool:
    """Whitespace as diff-document.md 7.2 defines it: U+0009, U+000B, U+000C, U+0020."""
    return ch in WHITESPACE


def leading_ws(line: str) -> str:
    i = 0
    while i < len(line) and is_ws(line[i]):
        i += 1
    return line[:i]


def key(line: str) -> str:
    """Move key (section 4.4) under ignore.whitespace "none": leading whitespace removed."""
    return line[len(leading_ws(line)):]


def mask_line(line: str, patterns: list[re.Pattern]) -> str:
    """Apply ignore.regex as diff-document.md 7.2.1 does (one placeholder per match).

    Python's re stands in for the Rust regex dialect; the corpus uses patterns
    that mean the same in both.
    """
    kept: list[tuple[int, int, int]] = []
    for index, pattern in enumerate(patterns):
        for m in pattern.finditer(line):
            if m.start() == m.end():
                continue
            if any(s < m.end() and m.start() < e for s, e, _ in kept):
                continue
            kept.append((m.start(), m.end(), index))
    out, pos = [], 0
    for start, end, index in sorted(kept):
        out.append(line[pos:start])
        out.append(chr(PLACEHOLDER_BASE + index))
        pos = end
    out.append(line[pos:])
    return "".join(out)


WORD_RE = re.compile(r"\w+|[\U000F0000-\U000F00FF]")


def words(line: str) -> list[str]:
    """Words of a move key (section 4.5): a stand-in for UAX #29 word-like segments."""
    return WORD_RE.findall(key(line))


def alnum(line: str) -> int:
    return sum(1 for ch in line if ch.isalnum())


def trivial(line: str) -> bool:
    return alnum(line) <= TRIVIAL_LINE_MAX_ALNUM


def meets_min_size(lines: list[str]) -> bool:
    total = sum(alnum(line) for line in lines)
    nontrivial = [line for line in lines if not trivial(line)]
    if total < MOVE_MIN_ALNUM:
        return False
    if len(nontrivial) >= MOVE_MIN_NONTRIVIAL_LINES:
        return True
    return len(nontrivial) == 1 and alnum(nontrivial[0]) >= MOVE_SINGLE_LINE_MIN_ALNUM


def indent_delta(left: str, right: str):
    """Return the indentation delta of a line pair, None for a blank pair, or False."""
    if key(left) == "" and key(right) == "":
        return None
    li, ri = leading_ws(left), leading_ws(right)
    if ri.endswith(li):
        added = ri[: len(ri) - len(li)]
        return ("", "") if added == "" else ("+", added)
    if li.endswith(ri):
        return ("-", li[: len(li) - len(ri)])
    return False


def levenshtein(a: str, b: str) -> int:
    if len(a) < len(b):
        a, b = b, a
    previous = list(range(len(b) + 1))
    for i, ca in enumerate(a, start=1):
        current = [i]
        for j, cb in enumerate(b, start=1):
            current.append(min(previous[j] + 1, current[j - 1] + 1, previous[j - 1] + (ca != cb)))
        previous = current
    return previous[-1]


def line_sim(a: str, b: str) -> int:
    ka, kb = key(a), key(b)
    m = max(len(ka), len(kb))
    if m == 0:
        return 10000
    return (10000 * (m - levenshtein(ka, kb))) // m


def block_similarity(left: list[str], right: list[str]):
    """Best monotone pairing (section 4.5). Returns (blockSim, pairs)."""
    n, m = len(left), len(right)
    sims = [[line_sim(left[i], right[j]) for j in range(m)] for i in range(n)]
    best = [[0] * (m + 1) for _ in range(n + 1)]
    for i in range(1, n + 1):
        for j in range(1, m + 1):
            options = [best[i - 1][j], best[i][j - 1]]
            if sims[i - 1][j - 1] >= MOVE_LINE_PAIR_MIN_BP:
                options.append(best[i - 1][j - 1] + sims[i - 1][j - 1])
            best[i][j] = max(options)
    pairs = []
    i, j = n, m
    while i > 0 and j > 0:
        s = sims[i - 1][j - 1]
        if s >= MOVE_LINE_PAIR_MIN_BP and best[i][j] == best[i - 1][j - 1] + s:
            pairs.append((i - 1, j - 1, s))
            i, j = i - 1, j - 1
        elif best[i][j] == best[i - 1][j]:
            i -= 1
        else:
            j -= 1
    pairs.reverse()
    n_left = sum(1 for line in left if not trivial(line))
    n_right = sum(1 for line in right if not trivial(line))
    total = sum(s for li, _, s in pairs if not trivial(left[li]))
    if n_left + n_right == 0:
        return 0, pairs
    return (2 * total) // (n_left + n_right), pairs


def word_similarity(left: list[str], right: list[str], pairs) -> int:
    """Word similarity of section 4.5 in basis points over the given pairs."""
    common = 0
    for li, ri, _ in pairs:
        if trivial(left[li]) or trivial(right[ri]):
            continue
        rest = list(words(right[ri]))
        for w in words(left[li]):
            if w in rest:
                rest.remove(w)
                common += 1
    total = sum(len(words(x)) for x in left + right if not trivial(x))
    return (20000 * common) // total if total else 0


# --- stand-in line diff -----------------------------------------------------

HUNK_RE = re.compile(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")


def git_change_ops(left: list[str], right: list[str]):
    """Return (deleted, inserted): maps of line number -> change op index.

    The lines are written with LF terminators and diffed by git.
    """
    with tempfile.TemporaryDirectory() as work:
        left_path, right_path = Path(work) / "left.txt", Path(work) / "right.txt"
        for path, lines in ((left_path, left), (right_path, right)):
            path.write_bytes("".join(line + "\n" for line in lines).encode("utf-8"))
        cmd = [
            "git", "-c", "core.autocrlf=false", "-c", "diff.noprefix=false",
            "diff", "--no-index", "--no-color", "--no-ext-diff", "--text",
            "--histogram", "--indent-heuristic", "-U0",
            str(left_path), str(right_path),
        ]
        proc = subprocess.run(cmd, capture_output=True, cwd=work)
    if proc.returncode not in (0, 1):
        raise RuntimeError(proc.stderr.decode("utf-8", "replace"))
    deleted: dict[int, int] = {}
    inserted: dict[int, int] = {}
    op = 0
    for raw in proc.stdout.split(b"\n"):
        match = HUNK_RE.match(raw.decode("utf-8", "replace"))
        if not match:
            continue
        a, b_, c, d = match.groups()
        a, c = int(a), int(c)
        b_ = 1 if b_ is None else int(b_)
        d = 1 if d is None else int(d)
        for line in range(a, a + b_):
            deleted[line] = op
        for line in range(c, c + d):
            inserted[line] = op
        op += 1
    return deleted, inserted


# --- reference-style pure move detection ------------------------------------


def detect_pure_moves(
    left: list[str], right: list[str], deleted: dict, inserted: dict, sizes: tuple[list[str], list[str]]
):
    """Pure moves over the masked lines `left` and `right`; `sizes` holds the
    original lines, whose alphanumerics count for the minimum size (4.2)."""
    size_left, size_right = sizes

    def pair_ok(i: int, j: int) -> bool:
        return (
            i in deleted
            and j in inserted
            and deleted[i] != inserted[j]
            and key(left[i - 1]) == key(right[j - 1])
            and indent_delta(left[i - 1], right[j - 1]) is not False
        )

    runs: list[tuple[int, int, int]] = []
    seen: set[tuple[int, int]] = set()
    for i in sorted(deleted):
        for j in sorted(inserted):
            if (i, j) in seen or not pair_ok(i, j):
                continue
            delta = None
            k = 0
            while pair_ok(i + k, j + k):
                d = indent_delta(left[i + k - 1], right[j + k - 1])
                if d is not None:
                    if delta is None:
                        delta = d
                    elif d != delta:
                        break
                seen.add((i + k, j + k))
                k += 1
            runs.append((i, j, k))

    claimed_left: set[int] = set()
    claimed_right: set[int] = set()
    moves = []
    while True:
        candidates = []
        for i, j, k in runs:
            t = 0
            while t < k:
                if (i + t) in claimed_left or (j + t) in claimed_right:
                    t += 1
                    continue
                start = t
                while t < k and (i + t) not in claimed_left and (j + t) not in claimed_right:
                    t += 1
                candidates.append((i + start, j + start, t - start))
        best = None
        for i, j, k in candidates:
            block = size_left[i - 1 : i - 1 + k]
            rblock = size_right[j - 1 : j - 1 + k]
            if not (meets_min_size([key(x) for x in block]) and meets_min_size([key(x) for x in rblock])):
                continue
            rank = (-sum(alnum(x) for x in block), -k, i, j)
            if best is None or rank < best[0]:
                best = (rank, i, j, k)
        if best is None:
            break
        _, i, j, k = best
        claimed_left.update(range(i, i + k))
        claimed_right.update(range(j, j + k))
        moves.append({"left": {"start": i, "count": k}, "right": {"start": j, "count": k}})
    moves.sort(key=lambda m: (m["left"]["start"], m["right"]["start"]))
    return moves


def line_set(rng: dict) -> set[int]:
    return set(range(rng["start"], rng["start"] + rng["count"]))


def jaccard(a: dict, b: dict) -> float:
    sa, sb = line_set(a), line_set(b)
    return len(sa & sb) / len(sa | sb)


def match(predictions: list[dict], labels: list[dict]):
    """Greedy one-to-one matching of section 5.3. Returns {label index: prediction index}."""
    scored = []
    for li, label in enumerate(labels):
        for pi, pred in enumerate(predictions):
            jl = jaccard(pred["left"], label["left"])
            jr = jaccard(pred["right"], label["right"])
            if jl >= MATCH_MIN_JACCARD and jr >= MATCH_MIN_JACCARD:
                scored.append((-min(jl, jr), li, pi))
    scored.sort()
    used_labels: set[int] = set()
    used_preds: set[int] = set()
    result = {}
    for _, li, pi in scored:
        if li in used_labels or pi in used_preds:
            continue
        used_labels.add(li)
        used_preds.add(pi)
        result[li] = pi
    return result


# --- options ----------------------------------------------------------------


def check_options(
    path: Path, errors: CaseErrors, *, detect_moves: bool | None, line_only: bool
) -> None:
    try:
        options = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        errors.error(f"options.json unreadable: {exc}")
        return
    allowed = {"algorithm", "granularity", "ignore", "context", "positionEncoding", "detectMoves", "adapter", "tree"}
    if not isinstance(options, dict) or set(options) - allowed:
        errors.error(f"options.json has unknown keys: {sorted(set(options) - allowed)}")
        return
    for name, values in OPTION_ENUMS.items():
        if name in options and options[name] not in values:
            errors.error(f"options.{name} = {options[name]!r} is not a valid value")
    if "positionEncoding" not in options:
        errors.error("options.json must pin positionEncoding")
    if line_only and options.get("granularity", "line") != "line":
        errors.error("moved cases must use granularity 'line'")
    ignore = options.get("ignore", {})
    if not isinstance(ignore, dict) or set(ignore) - {"whitespace", "case", "blankLines", "regex"}:
        errors.error("options.ignore has unknown keys")
    else:
        if ignore.get("whitespace", "none") not in WHITESPACE_MODES:
            errors.error("options.ignore.whitespace is not a valid value")
        if not isinstance(ignore.get("case", False), bool):
            errors.error("options.ignore.case must be a boolean")
        if not isinstance(ignore.get("blankLines", False), bool):
            errors.error("options.ignore.blankLines must be a boolean")
        regex = ignore.get("regex", [])
        if not isinstance(regex, list) or not all(isinstance(r, str) for r in regex):
            errors.error("options.ignore.regex must be an array of strings")
    context = options.get("context", 3)
    if not isinstance(context, int) or isinstance(context, bool) or not 0 <= context <= 2**31 - 1:
        errors.error("options.context must be an integer from 0 to 2^31 - 1")
    if detect_moves is not None and options.get("detectMoves", False) is not detect_moves:
        errors.error(f"options.detectMoves must be {str(detect_moves).lower()} here")


# --- per-case checks --------------------------------------------------------


def check_files(raw: bytes, side: str, errors: CaseErrors):
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        errors.error(f"{side}.txt is not valid UTF-8")
        return None
    if raw.startswith(b"\xef\xbb\xbf"):
        errors.error(f"{side}.txt starts with a BOM")
    if not text.endswith("\n"):
        errors.error(f"{side}.txt has no final newline")
    if re.search("\r(?!\n)", text):
        errors.error(f"{side}.txt contains a lone CR")
    lines = split_lines(text)
    for number, line in enumerate(lines, start=1):
        indent = leading_ws(line)
        if indent and key(line) and (set(indent) != {" "} or len(indent) % 2):
            errors.error(f"{side}.txt line {number}: indentation is not 2-space based")
    return lines


def check_range(rng, label: str, line_count: int, errors: CaseErrors) -> bool:
    if not isinstance(rng, dict) or set(rng) != {"start", "count"}:
        errors.error(f"{label}: must be an object with exactly start and count")
        return False
    start, count = rng["start"], rng["count"]
    if not all(isinstance(v, int) and not isinstance(v, bool) for v in (start, count)):
        errors.error(f"{label}: start and count must be integers")
        return False
    if start < 1 or count < 1:
        errors.error(f"{label}: start and count must be >= 1")
        return False
    if start + count - 1 > line_count:
        errors.error(f"{label}: range ends at line {start + count - 1}, file has {line_count} lines")
        return False
    return True


def check_case(directory: Path, totals: dict) -> CaseErrors:
    errors = CaseErrors(directory.name)
    # left.<ext> and right.<ext> (spec 5.1).
    def side_file(stem: str) -> Path:
        found = sorted(directory.glob(stem + ".*"))
        return found[0] if len(found) == 1 else directory / (stem + ".txt")

    left_path, right_path = side_file("left"), side_file("right")
    for required in (left_path, right_path, directory / "options.json", directory / "labels.json"):
        if not required.is_file():
            errors.error(f"missing {required.name}")
    if errors.errors:
        return errors
    # Edited cases may pin inline spans with another granularity (spec 5.1).
    check_options(directory / "options.json", errors, detect_moves=True, line_only=False)
    left = check_files(left_path.read_bytes(), "left", errors)
    right = check_files(right_path.read_bytes(), "right", errors)
    if left is None or right is None:
        return errors
    try:
        ignore = json.loads((directory / "options.json").read_text(encoding="utf-8")).get("ignore", {})
        patterns = [re.compile(p) for p in ignore.get("regex", [])]
    except (ValueError, re.error) as exc:
        errors.error(f"options.ignore.regex cannot be used here: {exc}")
        return errors
    if ignore.get("case", False):
        errors.error("moved cases must use ignore.case false")
    # Masked lines (move keys and the stand-in diff) next to the originals
    # (alphanumeric counts and indentation).
    mleft = [mask_line(line, patterns) for line in left]
    mright = [mask_line(line, patterns) for line in right]

    try:
        labels = json.loads((directory / "labels.json").read_text(encoding="utf-8"))
    except ValueError as exc:
        errors.error(f"labels.json is not valid JSON: {exc}")
        return errors
    if not isinstance(labels, dict) or "moves" not in labels or set(labels) - {"moves", "notes"}:
        errors.error("labels.json must have exactly 'moves' and optionally 'notes'")
        return errors
    if "notes" in labels and not isinstance(labels["notes"], str):
        errors.error("labels.notes must be a string")
    moves = labels["moves"]
    if not isinstance(moves, list):
        errors.error("labels.moves must be an array")
        return errors
    # The stand-in checks below compare lines under ignore.whitespace "none".
    # A negative case may use another mode (the engine must still find
    # nothing); the stand-in then checks the same inputs under "none".
    if ignore.get("whitespace", "none") != "none" and moves:
        errors.error("moved cases with labels must use ignore.whitespace 'none'")

    valid = []
    for index, move in enumerate(moves):
        where = f"moves[{index}]"
        if not isinstance(move, dict) or set(move) != {"left", "right", "kind"}:
            errors.error(f"{where}: must have exactly left, right and kind")
            continue
        if move["kind"] not in ("pure", "edited"):
            errors.error(f"{where}: kind must be 'pure' or 'edited'")
            continue
        ok = check_range(move["left"], f"{where}.left", len(left), errors)
        ok = check_range(move["right"], f"{where}.right", len(right), errors) and ok
        if ok:
            valid.append(move)
    order = [(m["left"]["start"], m["right"]["start"]) for m in valid]
    if order != sorted(order):
        errors.error("moves are not sorted by left.start, then right.start")
    for side in ("left", "right"):
        claimed: set[int] = set()
        for move in valid:
            lines = line_set(move[side])
            if lines & claimed:
                errors.error(f"two labels overlap on the {side} side")
            claimed |= lines

    deleted, inserted = git_change_ops(mleft, mright)

    for index, move in enumerate(valid):
        where = f"moves[{index}]"
        lslice = slice(move["left"]["start"] - 1, move["left"]["start"] - 1 + move["left"]["count"])
        rslice = slice(move["right"]["start"] - 1, move["right"]["start"] - 1 + move["right"]["count"])
        lblock, rblock = left[lslice], right[rslice]
        mlblock, mrblock = mleft[lslice], mright[rslice]
        for side, block in (("left", lblock), ("right", rblock)):
            if not meets_min_size(block):
                errors.error(f"{where}: {side} block is below the minimum move size (4.2)")
            if trivial(block[0]) or trivial(block[-1]):
                errors.info.append(f"{where}: {side} block starts or ends with a trivial line")
        if move["kind"] == "pure":
            if len(lblock) != len(rblock):
                errors.error(f"{where}: pure move with different line counts")
            else:
                deltas = set()
                for offset, (a, b_) in enumerate(zip(lblock, rblock)):
                    if key(mlblock[offset]) != key(mrblock[offset]):
                        errors.error(f"{where}: line pair {offset + 1} differs beyond indentation")
                    d = indent_delta(a, b_)
                    if d is False:
                        errors.error(f"{where}: line pair {offset + 1} has an incompatible indent")
                    elif d is not None:
                        deltas.add(d)
                if len(deltas) > 1:
                    errors.error(f"{where}: indentation delta changes inside a pure move")
        else:
            sim, pairs = block_similarity(mlblock, mrblock)
            paired_left = {p[0] for p in pairs}
            paired_right = {p[1] for p in pairs}
            if sim < MOVE_EDITED_MIN_SIM_BP:
                errors.error(f"{where}: edited block similarity {sim} bp is below {MOVE_EDITED_MIN_SIM_BP}")
            word_sim = word_similarity(mlblock, mrblock, pairs)
            if word_sim < MOVE_EDITED_MIN_WORD_SIM_BP:
                errors.error(
                    f"{where}: edited word similarity {word_sim} bp is below {MOVE_EDITED_MIN_WORD_SIM_BP}"
                )
            if 0 not in paired_left or len(lblock) - 1 not in paired_left:
                errors.error(f"{where}: left range must start and end with paired lines")
            if 0 not in paired_right or len(rblock) - 1 not in paired_right:
                errors.error(f"{where}: right range must start and end with paired lines")
            unpaired = len(lblock) - len(paired_left) + len(rblock) - len(paired_right)
            if sim == 10000 and unpaired == 0:
                errors.error(f"{where}: labeled edited but the block is pure")
            errors.info.append(f"{where}: edited blockSim {sim} bp, word similarity {word_sim} bp")
        left_lines = line_set(move["left"])
        right_lines = line_set(move["right"])
        if not left_lines <= set(deleted):
            errors.error(f"{where}: some left lines are not deleted by the stand-in diff")
        if not right_lines <= set(inserted):
            errors.error(f"{where}: some right lines are not inserted by the stand-in diff")
        left_ops = {deleted[n] for n in left_lines if n in deleted}
        right_ops = {inserted[n] for n in right_lines if n in inserted}
        if left_ops & right_ops:
            errors.error(f"{where}: left and right lines share a change op (rule 4.1.4)")

    predictions = detect_pure_moves(mleft, mright, deleted, inserted, (left, right))
    pure_labels = [m for m in valid if m["kind"] == "pure"]
    edited_labels = [m for m in valid if m["kind"] == "edited"]
    matched = match(predictions, pure_labels)
    for li, label in enumerate(pure_labels):
        if li not in matched:
            totals["fn"] += 1
            errors.error(f"pure label {label['left']} -> {label['right']} is not found by the stand-in detector")
        else:
            totals["tp"] += 1
            pred = predictions[matched[li]]
            if pred["left"] == label["left"] and pred["right"] == label["right"]:
                totals["exact"] += 1
            else:
                errors.info.append(f"boundary differs: label {label} vs predicted {pred}")
    used = set(matched.values())
    leftovers = []
    for pi, pred in enumerate(predictions):
        if pi in used:
            continue
        # A pure piece inside an edited label is joined into that edited move
        # (moves spec 4.3 step 4), so it is expected, not a false positive.
        # The stand-in does not trim blank edge lines (4.3 step 1), so they
        # do not count here.
        def non_blank(rng: dict, lines: list[str]) -> set[int]:
            return {n for n in line_set(rng) if lines[n - 1].strip()}

        inside = any(
            non_blank(pred["left"], left) <= line_set(label["left"])
            and non_blank(pred["right"], right) <= line_set(label["right"])
            for label in edited_labels
        )
        if inside:
            errors.info.append(f"pure piece {pred} inside an edited label")
        else:
            leftovers.append(pred)
    explained_by_edited = match(leftovers, edited_labels)
    for pi, pred in enumerate(leftovers):
        totals["fp"] += 1
        if pi in explained_by_edited.values():
            errors.error(f"a pure sub-run {pred} is found inside an edited label; redesign the case")
        else:
            errors.error(f"stand-in detector reports an unlabeled move {pred}")
    if not valid and predictions:
        errors.error("negative case, but moves were detected")
    totals["labels"] += len(valid)
    totals["pure"] += len(pure_labels)
    totals["edited"] += len(edited_labels)
    totals["negative"] += not valid
    return errors


def check_text_cases() -> list[CaseErrors]:
    results = []
    if not TEXT.is_dir():
        return results
    for directory in sorted(p for p in TEXT.iterdir() if p.is_dir()):
        errors = CaseErrors(f"text/{directory.name}")
        names = {p.name for p in directory.iterdir()}
        if not any(n.startswith("left.") for n in names) or not any(n.startswith("right.") for n in names):
            errors.error("missing left.* or right.*")
        if "case.md" not in names:
            errors.error("missing case.md")
        if "options.json" not in names:
            errors.error("missing options.json")
        else:
            check_options(directory / "options.json", errors, detect_moves=False, line_only=False)
        results.append(errors)
    return results


def main() -> int:
    verbose = "-v" in sys.argv[1:]
    totals = {"tp": 0, "fp": 0, "fn": 0, "exact": 0, "labels": 0, "pure": 0, "edited": 0, "negative": 0}
    results = [check_case(d, totals) for d in sorted(p for p in MOVED.iterdir() if p.is_dir())]
    text_results = check_text_cases()
    failed = 0
    for result in results + text_results:
        status = "FAIL" if result.errors else "ok"
        failed += bool(result.errors)
        if result.errors or verbose:
            print(f"{status:4} {result.name}")
            for message in result.errors:
                print(f"     error: {message}")
            if verbose:
                for message in result.info:
                    print(f"     info: {message}")
    tp, fp, fn = totals["tp"], totals["fp"], totals["fn"]
    precision = tp / (tp + fp) if tp + fp else 1.0
    recall = tp / (tp + fn) if tp + fn else 1.0
    print(
        f"moved cases: {len(results)} ({totals['negative']} negative), labels: {totals['labels']} "
        f"({totals['pure']} pure, {totals['edited']} edited); text cases: {len(text_results)}"
    )
    print(
        f"stand-in pure detector: precision {precision:.3f}, recall {recall:.3f}, "
        f"exact boundaries {totals['exact']}/{tp}"
    )
    if failed:
        print(f"{failed} case(s) failed")
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
