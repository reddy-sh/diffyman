# Text granularity and moved blocks (spec v0)

Status: draft for Phase 0. Defines the text adapter's inline refinement
(`granularity`), moved-block detection (`moves[]`) and their acceptance tests.
The Diff Document shape lives in `docs/spec/diff-document.md`. Algorithm IDs:
`text.refine@1`, `text.moves@1`; any output change bumps the matching ID.
MUST and MUST NOT are used as in RFC 2119.

## 1. Shared definitions

- **Line.** A line is the text between line terminators. Terminators are `LF`
  and `CRLF`. A lone `CR` is ordinary content. Terminators are never part of a
  line's content for comparison in this spec. Lines are numbered from 1.
- **Range.** `{start, count}`: `start` is a 1-based line number, `count >= 1`.
  It covers lines `start .. start+count-1`. This equals the half-open range
  `[start, start+count)` used elsewhere in the Diff Document.
- **Grapheme.** An extended grapheme cluster (UAX #29).
- **Word segment.** A segment produced by UAX #29 word boundaries (no
  dictionary). Whitespace and punctuation runs are segments too. A segment is
  **word-like** if it contains at least one alphanumeric code point.
- **Alphanumeric.** A code point for which Rust `char::is_alphanumeric()` is
  true (Unicode `Alphabetic` or `Numeric`). This is wider than git, which counts
  ASCII bytes only. CJK ideographs count.
- **Alnum count** of a line or block: the number of alphanumeric code points in
  its content.
- **Trivial line.** A line whose alnum count is `<= TRIVIAL_LINE_MAX_ALNUM`
  (for example `}`, `});`, `*/`, `fi`, a blank line).
- **Unicode version** is pinned per algorithm ID; updating it is an output
  change.
- **Exact arithmetic.** Every threshold test uses integers (cross-multiplication
  or basis points, 1 bp = 0.0001), never floating point. Reported similarities
  are basis points printed with exactly 4 decimals.

## 2. Pipeline order

1. Line diff (`text.histogram@1` by default) produces hunks and ops
   (`equal | insert | delete | replace`).
2. Moved-block detection (section 4) runs on deleted and inserted lines.
3. Inline refinement (section 3) runs on `replace` ops, with lines that belong
   to a move removed from both token streams.

Moves never change hunks or ops. A moved line stays inside its `delete`,
`insert` or `replace` op; `moves[]` adds the relation.

## 3. Granularity

Option `granularity`: `"line" | "word" | "char" | "smart"`. Default `"smart"`.

### 3.1 Eligibility (all modes except `line`)

Only `replace` ops get inline spans, and only when, after moved lines are
removed, both sides are non-empty and each side has at most `REFINE_MAX_LINES`
lines, `REFINE_MAX_BYTES` UTF-8 bytes and `REFINE_MAX_TOKENS` tokens. Pure
`insert` and `delete` ops never do. A too-large op gets `inline: []` and the
document gets one warning `text.refine.skipped_limit` with the skipped count.

A side's token stream is its lines in order. Each line terminator is one token
that equals any other terminator (EOL rules report those differences).

### 3.2 Modes

- `line`: no inline spans anywhere.
- `word`: tokens are word segments. Tokens compare after the active ignore
  rules (case folding under `ignoreCase`; under whitespace-ignore modes,
  whitespace tokens are normalized the same way as for line comparison).
- `char`: tokens are graphemes. A span never splits a grapheme.
- `smart`: chosen per op, section 3.3.

The token diff is a Myers diff over the token sequence (for example
`similar`'s slice diff). The only cleanup is the merge rule below.

Spans: a changed left token becomes a `delete` span, a changed right token an
`insert` span, with offsets per the Diff Document's `positionEncoding`.
Adjacent spans on the same side are merged. In `word` and
`smart`, two spans on the same side separated by exactly one whitespace token
are merged into one span covering it.

### 3.3 The `smart` rule

For each eligible `replace` op, in order:

1. **Spaceless scripts.** Let `G` be the non-whitespace graphemes of both sides.
   Let `S` be those whose first code point has Unicode Script in
   `SMART_SPACELESS_SCRIPTS`. If `100*|S| >= SMART_SPACELESS_MIN_PCT*|G|`, refine
   the op at `char` level, then go to step 4.
2. **Word pass.** Refine at `word` level.
3. **Sub-word pass.** For every change region of the word diff that replaces
   exactly one word-like token on the left with exactly one word-like token on
   the right (whitespace-free, no other tokens in the region), where both tokens
   have `>= SMART_SUBWORD_MIN_GRAPHEMES` graphemes: run a char diff of the two
   tokens. If `2*eq*100 >= SMART_SUBWORD_MIN_SIM_PCT*(lenL+lenR)` (`eq` = equal
   graphemes, `len` in graphemes), replace the two whole-token spans with the
   char-level spans. Otherwise keep the whole-token spans.
4. **Noise cutoff.** Let `L`, `R` be the non-whitespace graphemes per side and
   `eqL`, `eqR` those of them outside every span left by the steps above. If
   `(eqL+eqR)*100 < SMART_MIN_EQUAL_PCT*(L+R)`, emit `inline: []` for this op (the
   lines are shown as wholly changed, which reads better than scattered
   highlights).

The cutoff applies only to `smart`; explicit `word` and `char` always emit
spans for eligible ops.

### 3.4 Constants (`text.refine@1`)

| Name | Value |
|---|---|
| `REFINE_MAX_LINES` | 100 per side |
| `REFINE_MAX_BYTES` | 16384 per side |
| `REFINE_MAX_TOKENS` | 4096 per side |
| `SMART_SPACELESS_SCRIPTS` | Han, Hiragana, Katakana, Thai, Lao, Khmer, Myanmar |
| `SMART_SPACELESS_MIN_PCT` | 30 |
| `SMART_SUBWORD_MIN_GRAPHEMES` | 4 |
| `SMART_SUBWORD_MIN_SIM_PCT` | 50 |
| `SMART_MIN_EQUAL_PCT` | 25 |

Hangul is not in the spaceless set because Korean uses spaces between words.

### 3.5 Required smart test cases

Each is a golden case (`tests/corpus/smart-<name>/`):

| Case | Left -> right | Expected |
|---|---|---|
| `subword` | `colour` -> `color` | char span `u` (delete) |
| `identifier` | `getUserName()` -> `getUsername()` | char spans `N` / `n` |
| `short-word` | `foo bar` -> `foo baz` | word spans `bar` / `baz` (3 graphemes < 4) |
| `two-words` | `I ate a red apple` -> `I ate a green pear` | word spans `red apple` / `green pear`; merged across the space |
| `cjk` | `我喜欢猫` -> `我喜欢狗` | char spans `猫` / `狗` |
| `thai` | one Thai word changed | char spans |
| `noise` | two unrelated lines | `inline: []` |
| `emoji` | `👩‍💻 ok` -> `👨‍💻 ok` | one span per side covering the whole ZWJ cluster |
| `insert-only` | pure insert | no spans |
| `too-big` | replace of 101 lines | `inline: []` + `text.refine.skipped_limit` |
| `moved-in-replace` | replace op containing a moved block | moved lines carry no spans |

## 4. Moved blocks

Option `moveDetection`: `"off" | "pure" | "edited"`. MVP default `"pure"`;
default becomes `"edited"` when edited moves ship (v1). Option
`moveWhitespace`, section 4.4.

### 4.1 Definition

A **moved block** is a pair (left range, right range) where:

1. every left line is deleted (in a `delete` or `replace` op) and every right
   line is inserted (in an `insert` or `replace` op);
2. the left range and right range are each contiguous in line numbers;
3. no left line and no right line belongs to another move (moves are 1:1);
4. the left lines and the right lines are not in the same op (a deletion and an
   insertion inside one `replace` op is an in-place rewrite, not a move);
5. it meets the minimum size (4.2) on both sides;
6. for `kind: "pure"`, the lines match pairwise under `moveWhitespace`; for
   `kind: "edited"`, the block similarity is `>= MOVE_EDITED_MIN_SIM_BP`
   (4.5).

### 4.2 Minimum size

Both sides MUST have alnum count `>= MOVE_MIN_ALNUM` (20, as in git), **and**
either at least `MOVE_MIN_NONTRIVIAL_LINES` (2) non-trivial lines, or exactly
one non-trivial line with alnum count `>= MOVE_SINGLE_LINE_MIN_ALNUM` (40).

Why the line rule: git accepts single short lines, but lines such as
`return nil, err` repeat often in real code, so a one-line match is usually a
coincidence. Two non-trivial lines in the same order rarely match by chance.
Long single lines (an import list, a log message) are kept by the 40-alnum
exception. The precision target in section 5 depends on this rule.

Trivial lines may sit inside or at the edges of a block when they match, but
never count toward `MOVE_MIN_NONTRIVIAL_LINES`, and a block made only of trivial
lines is never a move.

### 4.3 Pure moves (MVP)

Line key: the line content after `moveWhitespace` normalization and, if
`ignoreCase` is on, case folding.

1. **Candidates.** All maximal pairs of runs `(l, r, k)` such that left lines
   `l..l+k-1` are deleted, right lines `r..r+k-1` are inserted, keys are equal
   pairwise, rule 4.1.4 holds, and the run cannot be extended up or down on
   both sides. A line whose key occurs more than `MOVE_MAX_KEY_OCCURRENCES`
   times among the deleted lines or among the inserted lines cannot start a
   candidate (it may still extend one).
2. **Selection.** Sort candidates by: alnum count descending, then `k`
   descending, then `l` ascending, then `r` ascending. Take the first one that
   meets 4.2 and claim its lines. Remove claimed lines from every remaining
   candidate; the leftover sub-runs become new candidates (re-sorted). Repeat
   until none qualifies.
3. **allowIndentationChange.** Keys compare with leading whitespace removed,
   and every non-blank line in the block MUST have the same indentation delta
   (one fixed string added to or removed from the start of the left indent;
   tabs and spaces compared literally). A candidate is split where the delta
   changes.

Guard: if deleted or inserted lines exceed `MOVE_MAX_CANDIDATE_LINES` on a side,
detection is skipped and the warning `text.moves.skipped_limit` is emitted.

### 4.4 Whitespace modes

`moveWhitespace`: `"exact" | "ignoreAtEol" | "ignoreChange" | "ignoreAll" |
"allowIndentationChange"`. They match git `--color-moved-ws` values `no`,
`ignore-space-at-eol`, `ignore-space-change`, `ignore-all-space`,
`allow-indentation-change`. Whitespace means Unicode `White_Space` minus line
terminators. `ignoreChange` collapses whitespace runs to one space and trims
the end. Default `allowIndentationChange` (git's default is `no`): code moved
into or out of a nested block is the most common real move. A looser
document-level whitespace ignore option overrides `moveWhitespace`.

### 4.5 Edited moves (v1)

Line similarity `sim(a, b)` in bp: `floor(10000*(m - d)/m)` where `d` is the
grapheme Levenshtein distance between the two keys and `m` the larger key
length in graphemes (two empty keys: 10000). Lines longer than
`MOVE_EDIT_MAX_LINE_GRAPHEMES` only pair when keys are equal.

Block similarity of a left range and a right range: align them with a
monotone pairing that maximizes the sum of `sim` over pairs, allowing only pairs
with `sim >= MOVE_LINE_PAIR_MIN_BP` (ties: the pairing with the
lexicographically smallest left indices). Then
`blockSim = floor(2*sum(sim of non-trivial pairs) / (nL + nR))`, with `nL`, `nR`
the non-trivial line counts.

Search, after pure moves are claimed, over unclaimed deleted and inserted lines
in different ops: seeds are pairs with `sim >= MOVE_EDIT_SEED_BP`. A seed grows
one step at a time by adding the next line above or below on the left, the
right, or both; it takes the step with the highest `blockSim` (ties: both, then
left, then right; below before above) while `blockSim >= MOVE_EDITED_MIN_SIM_BP`.
Final ranges MUST start and end with paired lines and MUST meet 4.2 on both
sides. Selection order is the same as 4.3 step 2, using the alnum count of
paired lines. A result with `blockSim = 10000` and no unpaired lines is
reported as `pure`. Blocks above `MOVE_EDITED_MAX_LINES` per side are not
searched.

### 4.6 What is not a move

- Lines in `equal` ops (blocks that stayed in place while others moved).
- Copies: inserted text whose source still exists unchanged on the left.
- Extra destinations of one deleted block: one pair wins, the rest stay inserts.
- Blocks below 4.2, trivial-only blocks (`}`, blank lines), and
  deletion/insertion pairs inside one op.
- Moves across files in a folder diff (out of scope for `text.moves@1`).

### 4.7 Output in `moves[]`

Each entry: `id`, `from: {side: "left", range}`, `to: {side: "right", range}`,
`kind: "pure" | "edited"`, `similarity` (4 decimals; `1.0000` for pure),
`fromOps` and `toOps` (lists of `{hunk, op}`: hunk id and 0-based op index of
every op the range touches, in order), and `editsRef` (`null` for pure; for
edited, a reference to inline spans between paired lines, computed with the
active granularity). Entries are sorted by `from.range.start`, then
`to.range.start`; ids are `m1`, `m2`, ... in that order. Zebra stripes are
derived by renderers, not stored: in `to.range.start` order, a move whose right
range directly follows the previous one gets the other stripe, else stripe 0.
`stats` adds `linesMoved` (left lines in moves).

### 4.8 Constants (`text.moves@1`)

| Name | Value |
|---|---|
| `TRIVIAL_LINE_MAX_ALNUM` | 2 |
| `MOVE_MIN_ALNUM` | 20 |
| `MOVE_MIN_NONTRIVIAL_LINES` | 2 |
| `MOVE_SINGLE_LINE_MIN_ALNUM` | 40 |
| `MOVE_MAX_KEY_OCCURRENCES` | 64 |
| `MOVE_MAX_CANDIDATE_LINES` | 50000 per side |
| `MOVE_EDITED_MIN_SIM_BP` | 8000 (0.80) |
| `MOVE_LINE_PAIR_MIN_BP` | 6000 |
| `MOVE_EDIT_SEED_BP` | 9000 |
| `MOVE_EDIT_MAX_LINE_GRAPHEMES` | 512 |
| `MOVE_EDITED_MAX_LINES` | 200 per side |

Where this text is ambiguous, the reference implementation in `diffyman-algo`
is normative for the current algorithm ID, and the ambiguity is fixed here.

## 5. Labeled moved-block corpus

### 5.1 Layout

`tests/corpus/moved/<case>/` holds `left.<ext>`, `right.<ext>`,
`options.json`, `labels.json`, and (from Phase 1) `expected.json`. Labels are
human ground truth; `expected.json` is the golden engine output. Metrics compare
predictions with labels, never with `expected.json`.

Minimum composition (>= 30 cases): >= 10 pure multi-line, >= 3 long single
line, >= 4 indentation change, >= 6 edited, >= 6 negative (`moves: []`:
trivial lines, in-place edits, copies, below threshold, repeated short lines),
>= 2 CJK or RTL, >= 2 CRLF, >= 2 with several moves (zebra).

### 5.2 `labels.json`

```json
{
  "moves": [
    { "left": { "start": 12, "count": 8 }, "right": { "start": 40, "count": 8 }, "kind": "pure" },
    { "left": { "start": 30, "count": 5 }, "right": { "start": 3, "count": 6 }, "kind": "edited" }
  ],
  "notes": "Function parse() moved below main(); helper moved up with one renamed variable."
}
```

Rules:

- Top level: an object with exactly `moves` (required array, may be empty)
  and `notes` (optional string). Unknown keys are a test-loader error.
- Each move: exactly `left`, `right`, `kind`. `left`/`right` are ranges
  (section 1): integers, `start >= 1`, `count >= 1`, within the file's line
  count, numbered by the line rule in section 1. `kind` is `"pure"` or
  `"edited"`.
- Moves are sorted by `left.start`, then `right.start`. Ranges of two labels
  MUST NOT overlap on the same side.
- Labelers mark what this spec says should be detected, with boundaries a
  reader would choose. Things a reader might call a move but that fall below
  section 4.2 belong in `notes`, not in `moves`.
- Files are UTF-8, 2-space indentation, trailing newline.

### 5.3 Matching and metrics

For a predicted move P and a label G, the per-side overlap is the Jaccard index
of their line sets: `|P ∩ G| / |P ∪ G|`. P **matches** G if the Jaccard index
is `>= MATCH_MIN_JACCARD` (0.5) on the left side **and** on the right side.
Matching is one-to-one per case: pairs are taken greedily by the larger
minimum-side Jaccard, ties by label order then prediction order.

Counts are summed over all cases (micro-average): TP = matched pairs,
FP = unmatched predictions, FN = unmatched labels. Precision = TP/(TP+FP),
recall = TP/(TP+FN).

Scope by mode: with `moveDetection: "pure"`, labels of kind `edited` are left
out of FN, and predictions matching them are left out of FP. With `"edited"`
all labels count.

### 5.4 Targets

| Gate | Mode | Precision | Recall | Other |
|---|---|---|---|---|
| Phase 1 | `pure` | >= 0.95 | >= 0.90 | every negative case yields zero moves |
| v1 | `edited` | >= 0.95 | >= 0.90 | kind accuracy on matched pairs >= 0.90 |

Tracked, not gating: exact-boundary rate (both ranges equal the label),
target >= 0.80. The runner prints metrics per case and in total and fails the
build when a gate is missed.
