# Golden test corpus

This directory holds the conformance and evaluation cases for diffyman. The specs
it follows are [`docs/spec/diff-document.md`](../../docs/spec/diff-document.md)
(the Diff Document, its options and the conformance rule in section 3) and
[`docs/spec/text-granularity-and-moves.md`](../../docs/spec/text-granularity-and-moves.md)
(moved-block detection and the labeled corpus in section 5).

## Layout

```
tests/corpus/
  text/NNN-slug/       40 text conformance cases
    left.<ext>         left input, byte-exact
    right.<ext>        right input, byte-exact
    options.json       options object (diff-document.md section 9)
    case.md            one paragraph: what the case tests, what a correct result shows
    expected.json      golden Diff Document (added later, see below)
  moved/NNN-slug/      41 labeled moved-block cases
    left.txt
    right.txt
    options.json       same shape, with "detectMoves": true
    labels.json        human ground truth (moves spec section 5.2)
    expected.json      golden Diff Document (added later)
  tools/
    make_text_cases.py      writes every text/ case
    make_moved_cases.py     writes every moved/ case and its labels
    check_labels.py         checks labels, inputs and options; must pass
    check_text_expected.py  re-derives text/ expected.json facts from the inputs; must pass
```

Cases are grouped by directory (`text/`, `moved/`, and later `smart-<name>/`
for smart-granularity cases). The conformance harness should walk every group
and treat any directory that holds `options.json` as a case. It passes the corpus file
names (`left.txt`, `left.py`, ...) as the input names, because names are part
of the document `id`.

## Generating the cases

Inputs are never edited by hand. They are produced by the scripts, which write
exact bytes, so CRLF terminators, a UTF-8 BOM, a missing final newline and
invisible characters survive. From the repository root:

```
uv run --python 3.12 python tests/corpus/tools/make_text_cases.py
uv run --python 3.12 python tests/corpus/tools/make_moved_cases.py
uv run --python 3.12 python tests/corpus/tools/check_labels.py
uv run --python 3.12 python tests/corpus/tools/check_text_expected.py
```

The generators rewrite inputs, `options.json`, `case.md` and `labels.json`, and
never touch `expected.json`. To change or add a case, edit its definition in
the generator, regenerate, and run the checker. `.gitattributes` and
`.editorconfig` in this directory stop git and editors from converting line
endings or trimming whitespace in the inputs.

`options.json` always spells out every key, even when it equals the default,
and always pins `positionEncoding`, so a case keeps its meaning if a default
changes. Most cases use `"granularity": "line"`. Cases 028 and 031-035 use
`word`, `char` and `smart` (moves spec section 3) and pin their inline spans;
the dedicated `smart-<name>/` cases of moves spec section 3.5 are still to come.

Inputs are UTF-8 except in two cases: `text/021-invalid-utf8` (Latin-1 bytes)
and `text/022-utf16-bom` (UTF-16LE with its BOM) are meant to fail decoding.
`text/028-long-line` holds one line of about 200 KB; every other input stays
under 20 KB.

## Expected results

`expected.json` is produced by the diffyman engine, then read and checked by a
person against the case's `case.md` (or `labels.json`) before it is committed.
It is regenerated, and reviewed again, whenever an algorithm ID changes
(diff-document.md section 12). Until then the cases hold inputs and options
only.

`check_text_expected.py` backs up that review for the `text/` cases. With its
own code it recomputes file meta, per-side warnings, hunk tiling, the context
rule, stats and the key equality of every `equal` line, and it turns every inline
span back into text. For the granularity cases it compares that text with a
table written by hand.

These consequences of schema 0.2.0 are deliberate:

- `text/021-invalid-utf8` and `text/022-utf16-bom` are `status: "error"`
  documents with an `invalid-utf8` error: there is no binary fallback yet.
- `text/018-log-timestamps-regex` and `text/038-regex-uuid-durations` mask
  timestamps, request ids and durations with `ignore.regex`
  (diff-document.md section 7.2.1). `text/039-regex-invalid` and
  `text/040-regex-limit-exceeded` are error documents (`invalid-regex`,
  `limit-exceeded`); 040 sits just above the pattern size limit, so it also
  checks that 32-bit WASM decides the same way as native builds.
- `text/036-blank-lines-ignored` runs the inputs of 026 with
  `ignore.blankLines: true` and is `identical`; `text/037-blank-lines-near-changes`
  shows `ignored` ops inside hunks next to real changes (section 7.3 step 3
  and 7.4).
- Every `moved/` case sets `"detectMoves": true` and has a golden
  `expected.json` with real `moves[]` (`text.moves@1`). Separately, the
  runner scores the predicted moves against `labels.json` (moves spec section
  5) and fails when the gate is missed. Never edit `labels.json` to make the
  metrics pass. `detectMoves` is the only move option (diff-document.md
  section 9): the detected kinds follow the moves algorithm ID, and move keys
  follow `ignore.whitespace` (moves spec section 4.4). Every moved case uses
  `ignore.whitespace: "none"`, so indentation changes are allowed with a
  constant delta and the rest of each line must match exactly.

## Labels

`labels.json` follows section 5.2 of the moves spec exactly: an object with
`moves` (ranges `{start, count}`, 1-based, and `kind` `pure` or `edited`) and
an optional `notes` string. Lines are numbered by the Diff Document rule
(diff-document.md section 7.2): LF, CRLF and a lone CR each end a line. The
moved inputs contain no lone CR (the checker rejects one, because its git
stand-in diff would treat it as content), so for these files the rule gives
the same numbering as splitting on LF and CRLF. Labels mark what the
spec says should be detected, with the boundaries a reader would choose; blank
separator lines are left out. Anything a reader might call a move but that is
below the minimum size (section 4.2) is explained in `notes`. Precision and
recall are computed against labels, never against `expected.json` (section 5.3).

The generator computes every label from the positions of named segments, so
line numbers are never counted by hand. `check_labels.py` then checks each
label against the files: shape and ranges, the minimum move size, pairwise
equality for pure moves, block similarity for edited moves, and consistency
with an independent histogram line diff (git's, used only as a stand-in) and a
small independent pure-move detector. Every negative case must yield no moves
in that detector, and every pure label must be found.

### Moved case mix

| Group | Cases | Count |
|---|---|---|
| Pure multi-line moves (up, down, small, large, swapped, several, reordered functions, one source with two destinations) | 001-018 | 18 |
| of which CRLF | 014, 015 | 2 |
| of which CJK or right-to-left | 016, 017 | 2 |
| of which several moves (zebra) | 011, 012, 013 | 3 |
| Long single-line moves | 019-021 | 3 |
| Indentation changes | 022-025 | 4 |
| Edited moves | 026-031 | 6 |
| Negative (`moves: []`): trivial lines, braces, short blocks, a short single line, delete plus unrelated insert, copies, duplicated text, in-place rewrite, repeated short lines | 032-040 | 9 |

That is 40 cases with 35 labeled moves (29 pure, 6 edited). Edited cases are
built so that no unchanged run inside the block meets the pure minimum size,
so the only sensible answer is one edited block.

## Originality

Every case in this corpus is original and written for diffyman. Inputs are
never copied from another product's test suite, and expected results are never
taken from another product's output: `expected.json` comes from this engine and
is checked by a person. The stand-in diff in `check_labels.py` only checks
that labels are consistent; it is never a source of expected output.
