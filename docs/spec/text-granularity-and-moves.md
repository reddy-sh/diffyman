# Text granularity and moved blocks (spec v0)

Status: draft for Phase 1. Defines how the text adapter computes inline
refinement (`granularity`) and moved blocks (`moves[]`), and their acceptance
tests. Algorithm IDs: `text.refine@1`, `text.moves@1`; any output change bumps
the matching ID. Neither ID is part of a release yet, so changes made before
the first release keep `@1`. MUST and MUST NOT are used as in RFC 2119.

[diff-document.md](diff-document.md) is the single normative source for every
output shape, option and warning code used here: the `inline` spans (§6.4),
the `moves[]` entries (§6.5), the options (§9) and the warning table (§10).
This file only says how the values are computed. Where the two disagree,
diff-document.md wins.

## 1. Shared definitions

- **Line.** As in diff-document.md §7.2: a line ends with a terminator (`LF`,
  `CRLF` or a lone `CR`) or with the end of input, and `CR` followed by `LF`
  is one `CRLF` terminator. Terminators are never part of a line's content
  for comparison in this spec. Lines are numbered from 1.
- **Whitespace.** As in diff-document.md §7.2: U+0009, U+000B, U+000C and
  U+0020 only.
- **Range.** A line range `{start, count}` as in diff-document.md §6.2. Move
  ranges and labels always have `count >= 1`, so a range covers lines
  `start .. start+count-1`.
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
  or basis points, 1 bp = 0.0001), never floating point. A reported similarity
  is a whole number of basis points written as a JSON number by the rules of
  diff-document.md §5 and §6.5: `1`, `0.875`, never `1.0000`.

## 2. Pipeline order

1. Line diff (`text.histogram@1` by default) produces hunks and ops
   (`equal | insert | delete | replace`, plus `ignored` under
   `ignore.blankLines`, diff-document.md §7.3).
2. Moved-block detection (section 4) runs on deleted and inserted lines.
3. Inline refinement (section 3) runs on `replace` ops, with lines that belong
   to a move removed from both token streams.

Moves never change hunks or ops. A moved line stays inside its `delete`,
`insert` or `replace` op; `moves[]` adds the relation.

## 3. Granularity

Option `granularity` (diff-document.md §9): `"line" | "word" | "char" |
"smart"`. Default `"line"`. All four values are supported under
`text.refine@1`. Golden cases in `tests/corpus/text/` cover `word`, `char` and
`smart` together with each `positionEncoding`; the dedicated smart cases of 3.5
are still to be added.

### 3.1 Eligibility (all modes except `line`)

Only `replace` ops get inline spans, and only when, after moved lines are
removed, both sides are non-empty and each side has at most `REFINE_MAX_LINES`
lines, `REFINE_MAX_BYTES` UTF-8 bytes and `REFINE_MAX_TOKENS` tokens. Pure
`insert` and `delete` ops never do. A too-large op gets `inline: []`, and the
document gets the warning `refine-skipped-limit` (diff-document.md §10): one
entry per document, however many ops were skipped, with no count.

A side's token stream is its lines in order. Each line terminator is one token
that equals any other terminator (EOL rules report those differences).

### 3.2 Modes

- `line`: no inline spans anywhere.
- `word`: tokens are word segments. Tokens compare after the active ignore
  rules (case folding under `ignore.case`; under `ignore.whitespace` modes
  other than `none`, whitespace tokens are normalized the same way as for
  line comparison).
- `char`: tokens are graphemes. A span never splits a grapheme.
- `smart`: chosen per op, section 3.3.

**Regex masks.** In every mode, each `ignore.regex` match kept by
diff-document.md §7.2.1 (found on the line content, with the same priority
rules) is one token. It equals a token of a match of the same pattern,
whatever the matched text, and nothing else. The text before, between and
after the matches of a line is segmented on its own, so a word or grapheme
never extends across a match. So a masked value that changed, such as a
timestamp on a line that changed for another reason, gets no span; a match
whose counterpart is missing, or belongs to another pattern, gets one span
that covers the whole match. Steps 1 and 4 of section 3.3 count the
graphemes of the content as it is, inside matches too.

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
| `too-big` | replace of 101 lines | `inline: []` + `refine-skipped-limit` |
| `moved-in-replace` | replace op containing a moved block | moved lines carry no spans |

## 4. Moved blocks

Option `detectMoves` (diff-document.md §9): boolean, default `false`. With
`true`, the engine runs the detection below and fills `moves[]`. There is no
other move option:

- **Kinds.** Which kinds are detected is fixed by the algorithm ID, not by an
  option. `text.moves@1` detects pure moves (4.3) and then edited moves (4.5).
- **Whitespace.** The move key (4.4) is derived from `ignore.regex`,
  `ignore.whitespace` and `ignore.case`, so there is no separate whitespace
  option for moves.

### 4.1 Definition

A **moved block** is a pair (left range, right range) where:

1. every left line is deleted (in a `delete` or `replace` op) and every right
   line is inserted (in an `insert` or `replace` op), and no line is in place
   (below);
2. the left range and right range are each contiguous in line numbers;
3. no left line and no right line belongs to another move (moves are 1:1);
4. the left lines and the right lines are not in the same **sub-op** (a
   deletion and an insertion at the same place is an in-place rewrite, not a
   move);
5. it meets the minimum size (4.2) on both sides;
6. for `kind: "pure"`, the lines have equal move keys pairwise (4.4) and, where
   4.4 requires it, the same indentation delta; for `kind: "edited"`, the
   block similarity is `>= MOVE_EDITED_MIN_SIM_BP` and the word similarity
   `>= MOVE_EDITED_MIN_WORD_SIM_BP` (4.5).

**Sub-ops.** An `insert` or `delete` op is one sub-op. A `replace` op is
aligned again over the move keys (4.4) of its own lines, with
`text.histogram@1`. Lines that this alignment pairs are **in place**: they
differ from their partner only in what the move key leaves out (indentation,
the terminator) and are never part of a move. Each maximal run of unpaired
lines of the op (on both sides, between two paired runs or an op edge) is one
sub-op. Without this step, a change that touches every line, such as a
whole-file conversion from LF to CRLF under `ignore.whitespace` `none`, would
put the whole file in one `replace` op and rule 4 would rule out every move.
With it, the lines that kept their place pair up and a block that moved is
still found. For a `replace` op whose lines have no move keys in common the
whole op is one sub-op, as before. "Op" in the rest of section 4 means sub-op.

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

Line key: the move key of 4.4.

1. **Candidates.** All maximal pairs of runs `(l, r, k)` such that left lines
   `l..l+k-1` are deleted, right lines `r..r+k-1` are inserted, keys are equal
   pairwise, rule 4.1.4 holds, and the run cannot be extended up or down on
   both sides. A run is a candidate only if it holds a **seed**: a line pair
   whose key occurs at most `MOVE_MAX_KEY_OCCURRENCES` times among the
   deleted lines and at most that many times among the inserted lines. Lines
   with more frequent keys (blank lines, `}`, `return err`) may still extend a
   run; they only cannot seed one. Blank lines at either end of a candidate
   (after the split of step 3) are trimmed; other trivial lines such as `}`
   stay. "Deleted" and "inserted" count every line of every sub-op. Lines
   in place (4.1) and lines of `ignored` ops are neither: they are never part
   of a move.
2. **Selection.** Sort candidates by: alnum count descending, then `k`
   descending, then `l` ascending, then `r` ascending. Take the first one that
   meets 4.2 and claim its lines. Remove claimed lines from every remaining
   candidate; the leftover sub-runs (blank ends trimmed again) become new
   candidates (re-sorted). Repeat until none qualifies. A candidate that fails
   4.2 is dropped: none of its sub-runs can pass.
3. **Indentation delta.** Where 4.4 requires it, every non-blank line in the
   block MUST have the same indentation delta: one fixed string added to the
   start of the left indent (the right indent ends with the left one) or
   removed from it (the left indent ends with the right one); tabs and spaces
   are compared literally, and equal indents are the empty delta. A candidate
   is split where the delta changes; blank lines between the two deltas belong
   to both parts before trimming. A line pair with no delta (neither indent
   ends with the other) is in no part.
4. **Joining.** A moved block with an edited line inside is found by step 2
   as two pure pieces, with the edited line in neither. So each selected
   pure move, in left-start order, walks down by the edge extension of 4.5
   step 2 (same step rule, ranking and `blockSim >= T` condition) inside its
   two ops: a step adds a pairable pair of unclaimed lines, or the next pure
   move of the same two ops when that move starts there; skipped lines must
   be unclaimed lines of the two ops. The walk stops when no step keeps
   `blockSim >= T`. The block then ends at the last pure move it absorbed
   whose lines, from the block start, are not all equal keys pairwise (so
   pieces split only by step 3 stay apart). If that block also meets the
   word similarity of 4.5 (its pairs are the line pairs of the absorbed pure
   moves and of the steps), it replaces the moves it absorbed and is `edited`
   with similarity `blockSim / 10000`. A pure move that absorbs no other
   move, or whose block misses the word similarity, is kept exactly as step 2
   selected it.

Guard: if deleted or inserted lines exceed `MOVE_MAX_CANDIDATE_LINES` on a side,
detection is skipped, `moves` is `[]` and the document gets the warning
`moves-skipped-limit` (diff-document.md §10).

Work: the seed cap bounds the seed pairs by `MOVE_MAX_KEY_OCCURRENCES` times
the number of deleted lines, and each maximal run is walked once, so a file
of many identical short lines costs linear work and yields no seeds.

### 4.4 Move key

The move key of a line is computed from its content (never its terminator):

1. Apply `ignore.regex` as diff-document.md §7.2.1 does: each kept match
   becomes the placeholder of its pattern.
2. Remove leading whitespace: the whitespace before the first other
   character or placeholder.
3. Apply `ignore.whitespace` to the rest as diff-document.md §7.2 step 3 does,
   without the terminator kind: `none` keeps the rest exactly, `trailing`
   removes trailing whitespace, `change` also collapses every other whitespace
   run to one U+0020, and `all` removes all whitespace. A placeholder is not
   whitespace.
4. If `ignore.case` is true, apply the same case folding as diff-document.md
   §7.2 step 4.

For line similarity (4.5) a placeholder is one grapheme and one word, equal
only to a placeholder of the same pattern; the text on either side of it is
segmented on its own.

Under `ignore.whitespace` `none` and `trailing`, a pure move also needs a
constant indentation delta (4.3 step 3). This is git's
`--color-moved-ws=allow-indentation-change`; with `trailing` it is combined
with `ignore-space-at-eol`. Under `change` and `all` the delta is not checked,
because those modes already ignore the amount of whitespace.

Why there is no separate option: code moved into or out of a nested block is
the most common real move, so indentation changes are always allowed (git's
default, `no`, would miss them). Every other rule follows the document's own
`ignore` options, regex masks included, so lines that are equal for the line
diff are also equal as move keys: a block of log lines that moved and got new
timestamps is a pure move when `ignore.regex` masks the timestamps. A
terminator change never makes two move keys differ, and it never prevents a
move: under `ignore.whitespace` `none` it does make the line diff report the
line as changed, but the sub-ops of 4.1 put lines that differ only in their
terminator back in place.

### 4.5 Edited moves

Edited moves are searched after the pure moves are claimed and joined (4.3
step 4), over the lines that are still unclaimed. A **segment** is a maximal
run of unclaimed lines of one sub-op (4.1) on one side; segments without a
non-trivial line are ignored.

**Line similarity** `sim(a, b)` in bp: `floor(10000*(m - d)/m)` where `d` is the
grapheme Levenshtein distance between the two move keys and `m` the larger key
length in graphemes (equal keys: 10000). Only non-trivial lines pair, and only
when they are in different ops. A pair is **pairable** when
`sim >= MOVE_LINE_PAIR_MIN_BP` and a **seed** when `sim >= MOVE_EDIT_SEED_BP`.

**Long lines.** When one of the two keys is longer than
`MOVE_EDIT_MAX_LINE_GRAPHEMES` graphemes, the pair is compared by removing
the common prefix of the two keys and then the common suffix of what is left
(in graphemes). Removing a common prefix or suffix never changes the
Levenshtein distance, so `d` is the distance of the two remainders and `sim`
is exact. The pair is pairable only when both remainders have at most
`MOVE_EDIT_MAX_LONG_REMAINDER` graphemes. So a long paragraph or log line
with a few small edits still pairs, at a cost of about the line length plus
`ceil(r/64)*r'` word steps for remainders of `r <= r'` graphemes, and lines
that differ throughout (beyond the length and histogram bounds below) cost
little.

**Word similarity.** The words of a move key are its word-like word segments
(section 1) and its placeholders (4.4). For a pair, `common` is the size of
the multiset intersection of the two keys' words. The word similarity of a
block is `floor(10000*2*sum(common) / W)`, the sum taken over its pairs, where
`W` counts the words of all non-trivial lines of both ranges, paired or not.
An edited move needs word similarity `>= MOVE_EDITED_MIN_WORD_SIM_BP`. Why:
grapheme similarity rewards shared structure. A config section moved with a
new value on every line (`max_requests = 2000` to `max_requests = 5000`)
has a block similarity over 0.9, but only one word in two survived on each
line, so its word similarity is about 0.5 and it is a removal and an
addition, not a move. A moved block with a renamed variable or a few typo
fixes keeps most of its words.

**Block similarity** of a left range and a right range with a monotone set of
pairable pairs: `blockSim = floor(2*sum(sim) / (nL + nR))`, where `nL`, `nR`
count the non-trivial lines of the ranges (paired or not). With
`T = MOVE_EDITED_MIN_SIM_BP`, the block **score**
`sum(2*sim - 2*T) - T*(non-trivial lines not in a pair)` is `>= 0` exactly
when `blockSim >= T`. Trivial lines score 0, paired or not.

**Candidates.** For every pair of segments in different ops that holds a seed,
and for every seed in it, the candidate is built in four steps:

1. **Core.** The monotone alignment through the seed, inside the two
   segments, with the highest score. Ties go to more pairs; the remaining ties
   are broken by a forward and a backward dynamic program over the segment
   pair that prefer, per cell, a pair, then an unpaired left line, then an
   unpaired right line. Seeds whose cores have the same first and last pair
   give one candidate (the seed with the best score, then the most pairs,
   then the first in left-then-right line order).
2. **Edge extension.** Repeatedly add one pairable pair just below or just
   above the block that skips at most `MOVE_EDIT_EXTEND_SKIP` lines on each
   side, as long as `blockSim` stays `>= T`. The step with the highest
   resulting `blockSim` wins (exact rational comparison); ties go to fewer
   skipped lines, then fewer skipped left lines, then below before above.
   This keeps a block whole when one line inside it changed more than the
   score alone would accept, for example a dropped comment line.
3. **Trivial edges.** Trivial lines with equal keys directly above or below
   the block extend it on both sides at once; blank lines at either end are
   then trimmed again.
4. The ranges start and end with pairs (after step 3, with matching trivial
   lines), MUST meet 4.2 on both sides, and have `blockSim >= T`. A block
   that is not reported as `pure` (see Kind) also needs the word similarity
   above.

**Selection.** Sort candidates by: alnum count of the paired left lines
descending, then number of pairs descending, then left start ascending, then
right start ascending, then shorter ranges, then the smaller seed. Take the
first one whose lines are all still unclaimed and claim them. A candidate that
lost a line to an earlier selection is rebuilt from its seed over the current
segments (and dropped if the seed itself was claimed); one that lost no line
stays as it is, because its core is still feasible in the smaller segments.
Repeat until no candidate is left.

**Kind.** A selected block whose two ranges have the same length, with keys
equal pairwise and one indentation delta where 4.4 requires it, is reported
as `pure` with similarity `1`. Every other block is `edited` with similarity
`blockSim / 10000`. That is `1` whenever every non-trivial line of the block
is paired with an equal key, even though the block is not pure: for example
when the indentation delta changes inside it, or when one side has an extra
trivial line (a blank line or a lone `}`), which scores 0 and does not count
in `nL + nR`.

**Work limits** (deterministic, no clock):

- If there are more than `MOVE_EDIT_MAX_PAIRS` segment pairs (left segments
  times right segments), the edited search is not run. Otherwise the
  segment pairs in different ops are taken by increasing number of line
  pairs (every line counts, trivial or not), then left start, then right
  start, and each one's line pairs are compared while the running total
  stays `<= MOVE_EDIT_MAX_PAIRS`. The first segment pair that does not fit,
  and every one after it, is not searched: none of its lines pair. So one
  large pair of unrelated ops does not switch off the search in small ones.
  In both cases the pure moves stay and the document gets
  `moves-skipped-limit`.
- Each sweep of a segment pair (initial or for a rebuild) counts two cells
  per line pair, and every traceback and extension step counts one. The
  first sweep that would take the count above `MOVE_EDIT_MAX_CELLS` is not
  made; from then on no segment pair is swept, candidates that need a
  rebuild are dropped, the moves already selected and the remaining valid
  candidates stay, and the document gets `moves-skipped-limit`.
- Line similarity uses a length bound (`|la - lb| <= 0.4 m`) and a grapheme
  histogram bound before the bit-parallel distance, so the cost per pair is
  small. All of these are exact filters: they never drop a pairable pair.
- Long-line pairs (see Long lines) are compared in a fixed order: the steps
  of 4.3 step 4 first, then the edited search by left line, then by right
  line. Each one counts the graphemes its prefix and suffix scans compared
  (plus one) and, when its remainders are compared, `ceil(r/64)*r'`. Once
  the count reaches `MOVE_EDIT_MAX_LONG_SCAN`, no further long-line pair is
  pairable and the document gets `moves-skipped-limit`; pairs of short lines
  are not affected.

### 4.6 What is not a move

- Lines in `equal` ops (blocks that stayed in place while others moved).
- Copies: inserted text whose source still exists unchanged on the left.
- Extra destinations of one deleted block: one pair wins, the rest stay inserts.
- Blocks below 4.2, trivial-only blocks (`}`, blank lines), and
  deletion/insertion pairs inside one sub-op (4.1), including lines that only
  changed their indentation or terminator in place.
- Blocks that share only their structure: lines that pair by grapheme
  similarity but lost most of their words (the word similarity of 4.5).
- Moves across files in a folder diff (out of scope for `text.moves@1`).

### 4.7 Output in `moves[]`

Each detected move becomes one `moves[]` entry with the shape, order and ids
of diff-document.md §6.5. This section only fills in the values:

- `from.range` and `to.range` are the selected left and right ranges.
- `kind: "pure"` with `similarity` `1` for a pure move (4.3, or a 4.5 result
  reported as pure). `kind: "edited"` with `similarity` = `blockSim / 10000`
  for an edited move.
- `fromOps` and `toOps` follow from the ranges and the hunks.

Derived data is not stored. Zebra stripes: in `to.range.start` order, a move
whose right range directly follows the previous one gets the other stripe,
else stripe 0. Moved line count: the sum of `from.range.count`.

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
| `MOVE_EDITED_MIN_WORD_SIM_BP` | 6000 (0.60) |
| `MOVE_LINE_PAIR_MIN_BP` | 6000 |
| `MOVE_EDIT_SEED_BP` | 9000 |
| `MOVE_EDIT_MAX_LINE_GRAPHEMES` | 256 |
| `MOVE_EDIT_MAX_LONG_REMAINDER` | 4096 graphemes per side |
| `MOVE_EDIT_MAX_LONG_SCAN` | 32000000 |
| `MOVE_EDIT_EXTEND_SKIP` | 2 lines per side |
| `MOVE_EDIT_MAX_PAIRS` | 1000000 |
| `MOVE_EDIT_MAX_CELLS` | 8000000 |

Where this text is ambiguous, the reference implementation in `diffyman-algo`
is normative for the current algorithm ID, and the ambiguity is fixed here.

## 5. Labeled moved-block corpus

### 5.1 Layout

`tests/corpus/moved/<case>/` holds `left.<ext>`, `right.<ext>`,
`options.json` (with `"detectMoves": true`), `labels.json` and
`expected.json`. Labels are
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
- Files are UTF-8, 2-space indentation, trailing newline, and no lone `CR`.
  A lone `CR` would still end a line by section 1, but the label checker's
  independent stand-in diff treats it as content, so the corpus avoids it.

### 5.3 Matching and metrics

For a predicted move P and a label G, the per-side overlap is the Jaccard index
of their line sets: `|P ∩ G| / |P ∪ G|`. P **matches** G if the Jaccard index
is `>= MATCH_MIN_JACCARD` (0.5) on the left side **and** on the right side.
Matching is one-to-one per case: pairs are taken greedily by the larger
minimum-side Jaccard, ties by label order then prediction order.

Counts are summed over all cases (micro-average): TP = matched pairs,
FP = unmatched predictions, FN = unmatched labels. Precision = TP/(TP+FP),
recall = TP/(TP+FN).

Scope: `text.moves@1` detects pure and edited moves, so all labels count. The
runner also prints a pure-only scope, where labels of kind `edited` are left
out of FN and predictions matching them are left out of FP.

**Kind accuracy** is the share of matched pairs whose predicted kind equals the
label's kind. The **exact-boundary rate** is the share of matched pairs whose
two ranges both equal the label's.

### 5.4 Targets

| Gate | Scope | Precision | Recall | Other |
|---|---|---|---|---|
| Phase 1 (`text.moves@1`) | all labels | >= 0.95 | >= 0.90 | every negative case (no labels) yields zero moves |
| v1 | all labels | >= 0.95 | >= 0.90 | kind accuracy >= 0.90 |

Tracked, not gating: kind accuracy before v1, and the exact-boundary rate,
target >= 0.80. A label may include a line that the line diff keeps in an
`equal` op (most often a closing brace that the diff slides to the other end
of a deleted block); no move can contain that line (4.1 rule 1), so such a
label can match but never exactly. The runner prints metrics per case, per
category (the word after the case number: `pure`, `single`, `indent`,
`edited`, `neg`) and in total, and fails the run when a gate is missed.
