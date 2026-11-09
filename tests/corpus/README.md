# Golden test corpus

This directory holds the conformance and evaluation cases for diffyman. The specs
it follows are [`docs/spec/diff-document.md`](../../docs/spec/diff-document.md)
(the Diff Document, its options and the conformance rule in section 3) and
[`docs/spec/text-granularity-and-moves.md`](../../docs/spec/text-granularity-and-moves.md)
(moved-block detection and the labeled corpus in section 5).

## Layout

```
tests/corpus/
  text/NNN-slug/       80 text conformance cases
    left.<ext>         left input, byte-exact
    right.<ext>        right input, byte-exact
    options.json       options object (diff-document.md section 9)
    case.md            one paragraph: what the case tests, what a correct result shows
    expected.json      golden Diff Document (added later, see below)
  moved/NNN-slug/      68 labeled moved-block cases
    left.<ext>         left.txt, or the language's extension (left.go, left.sql, ...)
    right.<ext>
    options.json       same shape, with "detectMoves": true
    labels.json        human ground truth (moves spec section 5.2)
    expected.json      golden Diff Document (added later)
  binary/NNN-slug/     29 binary and type-detection cases (diff-document.md sections 16, 17)
    left.<ext>         left input, byte-exact (PNG, GIF, PDF, WASM, SQLite, ELF, gzip, ZIP, random bytes, text)
    right.<ext>
    options.json
    case.md
    expected.json
  tree/NNN-slug/       39 folder and archive cases (diff-document.md section 19)
    left/ or left.<ext>    a folder, or an archive (zip, tar, tar.gz, tar.zst) or file
    right/ or right.<ext>
    options.json
    case.md
    expected.json
  tools/
    make_text_cases.py      writes every text/ case
    make_moved_cases.py     writes every moved/ case and its labels
    make_binary_cases.py    writes every binary/ case (random bytes from a fixed seed)
    make_tree_cases.py      writes every tree/ case (folders, and archives built byte by byte)
    check_labels.py         checks labels, inputs and options; must pass
    check_text_expected.py  re-derives text/ expected.json facts from the inputs; must pass
    check_binary_expected.py  re-derives binary/ expected.json facts from the inputs; must pass
```

Cases are grouped by directory (`text/`, `moved/`, `binary/`, `tree/`; the
smart-granularity cases of moves spec section 3.5 are `text/` cases). The
conformance harness
should walk every group and treat any directory that holds `options.json` as a
case. It passes the corpus file names (`left.txt`, `left.py`, ...) as the input
names, because names are part of the document `id`; a folder input (`left/`)
is diffed by path, and its name is the folder name.

## Generating the cases

Inputs are never edited by hand. They are produced by the scripts, which write
exact bytes, so CRLF terminators, a UTF-8 BOM, a missing final newline and
invisible characters survive. From the repository root:

```
uv run --python 3.12 python tests/corpus/tools/make_text_cases.py
uv run --python 3.12 python tests/corpus/tools/make_moved_cases.py
uv run --python 3.12 python tests/corpus/tools/check_labels.py
uv run --python 3.12 python tests/corpus/tools/check_text_expected.py
uv run --python 3.12 python tests/corpus/tools/make_binary_cases.py
uv run --python 3.12 python tests/corpus/tools/check_binary_expected.py
uv run --python 3.12 python tests/corpus/tools/make_tree_cases.py
```

The generators rewrite inputs, `options.json`, `case.md` and `labels.json`, and
never touch `expected.json`. To change or add a case, edit its definition in
the generator, regenerate, and run the checker. `.gitattributes` and
`.editorconfig` in this directory stop git and editors from converting line
endings or trimming whitespace in the inputs.

`options.json` always spells out every key, even when it equals the default
(the `tree` options included, which only change tree results), and always
pins `positionEncoding`, so a case keeps its meaning if a default changes. Most cases use `"granularity": "line"`. Cases 028, 031-035, 041, 045,
047, 048, 054, 055 and 065-080 use `word`, `char` and `smart` (moves spec
section 3) and pin their inline spans, which `check_text_expected.py` turns
back into text and compares with its hand-written table. Every granularity
other than `line` runs with every position encoding at least once. The smart
cases of moves spec section 3.5 are `text/` cases rather than a group of their
own: `subword` is 033, `noise` 034, `too-big` 035, and `identifier`,
`short-word`, `two-words`, `cjk`, `thai`, `emoji` and `insert-only` are
074-080; `moved-in-replace` is still to come. All three algorithms appear
(`myers` in 004, 047, 059 and 073, `patience` in 006, 046, 060 and 061), and
each ignore option alone and all of them together (064).

Text cases 044-073 are source code and data in many formats (Python,
JavaScript, TypeScript, Go, Rust, Java, C, C#, SQL, YAML, JSON, TOML,
Markdown, HTML, CSS, shell), named with their usual extension, so the
`mediaType` of each side comes from the extension table of
diff-document.md section 16.1 (`text/javascript`, `application/json`,
`text/markdown`, `text/html`, `text/css`, ...; `text/plain` for extensions
the table does not list). Where git has the same mode (any algorithm, with
`-b`, `-w` or `--ignore-blank-lines`), their hunks were also compared with
`git diff --no-index` during review and are the same.

Text inputs are UTF-8 except in two cases: `text/021-invalid-utf8` (Latin-1
bytes) and `text/022-utf16-bom` (UTF-16LE with its BOM) are meant to fail
decoding, so they pin `"adapter": "text"`. `text/028-long-line` holds one line
of about 200 KB, and the three large generated cases `text/058` (a log, about
81 KB per side), `text/059` (a CSV export, about 48 KB) and `text/060` (a
JavaScript module, about 82 KB) stay under 100 KB; every other text input
stays under 20 KB. The `binary/` inputs
are up to 256 KiB (`binary/013-chunked-two-edits`), large enough to reach the
chunked path of the binary diff, with one deliberate exception:
`binary/007-random-1mb-insertion` is 1 MiB on the left and 1 MiB plus 100
bytes on the right (about 2 MiB for the pair), the only corpus files over
1 MiB. The case checks that one small insertion is found exactly among about
128 first-level chunks of random data, which a smaller input would not show.
Both files are random bytes that `make_binary_cases.py` rebuilds from a fixed
xorshift seed, so they can be regenerated and checked at any time, and they
stay in the repository because strict conformance reads them.

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

These consequences of schema 0.3.0 are deliberate:

- `text/021-invalid-utf8` and `text/022-utf16-bom` pin `"adapter": "text"` and
  are `status: "error"` documents with an `invalid-utf8` error. Under the
  default `"adapter": "auto"` the same inputs are binary documents with a
  `binary-fallback` warning: `binary/009-latin1-vs-utf8` and
  `binary/010-utf16-vs-utf8`.
- `mediaType` is detected (diff-document.md section 16.1): `text/010-utf8-bom`
  (`.csv`) is `text/csv`, `text/029-trailing-whitespace` (`.yaml`) is
  `application/yaml`, and the Latin-1 side of 021 is
  `application/octet-stream`. Every other text input is `text/plain`.
- The `binary/` cases cover a PNG with one changed byte, inserted, truncated
  and identical binaries, text against binary, UTF-8 with NUL bytes, a 1 MiB
  random pair with an insertion, empty against binary, both forced adapters,
  the chunked path with two edits, the 8-byte join rule, an unrelated pair
  that hits the byte-alignment limit, media types from magic bytes, and
  overwrites closer together than one chunk (`binary/017-dense-overwrites`,
  aligned by the smaller chunk levels). Cases 018-029 add file formats whose
  media type comes from the magic number (GIF, PDF, WebAssembly, SQLite,
  ELF), a binary input whose media type comes from its extension because no
  magic matches (`.jpg`), a 64-byte header put in front, two empty inputs
  under `adapter: "binary"`, a binary input against an empty one, a 70000-byte
  cut found by the chunked path, a periodic input whose deletion has a
  canonical position, and valid UTF-8 that is binary because more than 10 %
  of its sniff window is control bytes.
  `check_binary_expected.py` rebuilds the right input from the left one and
  the ranges for every case and compares the exact ranges where the answer is
  known in advance.
- The `tree/` cases (kind `tree`, diff-document.md section 19) cover folders
  (changes with text and binary child documents, renames by content with the
  same-name tie-break, type changes, `tree.exclude`, `.ignore` files, move
  detection off, the child budget) and archives: a folder against a zip, tar
  against tar.gz and tar.zst (pax and GNU long names, links, a FIFO),
  traversal names (`../`, absolute, drive letter), a tiny zip bomb and a
  tar.gz bomb (`tree-limit`, status `partial`), overlapping zip entries
  that share one stored stream (`tree/027`, read once), duplicate names, encrypted
  entries, nested zips with and without the depth limit, and the error
  documents for a corrupt zip, a forged zip64 entry count, `adapter: "tree"`
  on text, a folder against a plain file and an invalid exclude pattern.
  Cases 030-039 add a folder of 21 files with scattered changes, a renamed
  folder (its files `moved`, the folders removed and added), deflated zip
  entries, a tar file that became a symbolic link (`typeChange`), globstar
  and negated `tree.exclude` patterns, a folder against a tar, the same
  files as a zip and a tar.gz (`identical`), text options applied to child
  documents (`ignore.whitespace: "all"` makes a modified file's child
  `identical`; `granularity: "word"` gives a child inline spans), and zip
  entries with UTF-8 names (flag bit 11).
  Folder inputs hold regular files only (git keeps no empty folders, and
  symbolic links are not portable); those and unreadable files are covered
  by the Rust tests of `diffyman-adapter-tree`. Ignore-file cases use
  `.ignore`, because a `.gitignore` in the corpus would hide fixture files
  from git. Every archive byte, deflate stream included, is written by the
  generator, so the fixtures do not depend on the local zlib.
- `text/018-log-timestamps-regex` and `text/038-regex-uuid-durations` mask
  timestamps, request ids and durations with `ignore.regex`
  (diff-document.md section 7.2.1). `text/039-regex-invalid` and
  `text/040-regex-limit-exceeded` are error documents (`invalid-regex`,
  `limit-exceeded`); 040 sits just above the pattern size limit, so it also
  checks that 32-bit WASM decides the same way as native builds.
  `text/041-regex-inline-spans` uses `word` granularity with masked
  timestamps and durations: masked values that changed get no inline span
  (moves spec section 3.2).
- `text/036-blank-lines-ignored` runs the inputs of 026 with
  `ignore.blankLines: true` and is `identical`; `text/037-blank-lines-near-changes`
  shows `ignored` ops inside hunks next to real changes (section 7.3 step 3
  and 7.4); in `text/043-blank-lines-slider-merge` slider compaction joins a
  blank deletion to a real one, so the blank line is counted, as in git
  (section 7.3.1 rule 4).
- Every `moved/` case sets `"detectMoves": true` and has a golden
  `expected.json` with real `moves[]` (`text.moves@1`). Separately, the
  runner scores the predicted moves against `labels.json` (moves spec section
  5) and fails when the gate is missed. Never edit `labels.json` to make the
  metrics pass. `detectMoves` is the only move option (diff-document.md
  section 9): the detected kinds follow the moves algorithm ID, and move keys
  follow `ignore.regex` and `ignore.whitespace` (moves spec section 4.4).
  Every labeled moved case uses `ignore.whitespace: "none"`, so indentation
  changes are allowed with a constant delta and the rest of each line must
  match exactly; `moved/042-pure-masked-timestamps` and
  `moved/046-edited-indent-masked-utf16` use `ignore.regex`, and
  `moved/044-pure-crlf-conversion` is the only one whose sides use different
  terminators (LF left, CRLF right). The negative case
  `moved/047-neg-reindented-check-whitespace-change` runs the inputs of 040
  under `ignore.whitespace: "change"` (re-indented swaps, moves spec 4.5).

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
in that detector, and every pure label must be found. The checker applies the
case's `ignore.regex` masks (with Python's `re`, which agrees with the Rust
dialect for the corpus patterns) and runs the stand-in diff on LF-terminated
copies, because move keys ignore terminators (moves spec section 4.1). Edited
labels must also reach the word similarity of section 4.5, with `\w+` runs
standing in for word segments.

### Moved case mix

| Group | Cases | Count |
|---|---|---|
| Pure multi-line moves (up, down, small, large, swapped, several, reordered functions, one source with two destinations, masked timestamps, a whole-file CRLF conversion; Go, Java, SQL, CSS, Markdown, YAML, C, shell, HTML and JSON; a 62-line class; a two-line block at exactly the minimum size) | 001-018, 042, 044, 048-055, 061, 062, 065, 066 | 32 |
| of which CRLF | 014, 015, 044, 054 | 4 |
| of which CJK or right-to-left | 016, 017 | 2 |
| of which several moves (zebra) | 011, 012, 013, 061 | 4 |
| Long single-line moves | 019-021 | 3 |
| Indentation changes (TypeScript into a namespace, Python out of a class) | 022-025, 056, 057 | 6 |
| Edited moves (one with a line over 256 graphemes, one with word spans in UTF-16 offsets; a renamed Rust field, a SQL column added, HTML classes renamed, one INI value changed) | 026-031, 041, 043, 046, 058-060, 067 | 13 |
| Negative (`moves: []`): trivial lines, braces, short blocks, a short single line, delete plus unrelated insert, copies, duplicated text, in-place rewrite, repeated short lines, a config section whose values all changed, a re-indented check under `ignore.whitespace: "change"`, reversed imports, a two-line block one alphanumeric character under the minimum size, a function deleted and written again | 032-040, 045, 047, 063, 064, 068 | 14 |

That is 68 cases with 59 labeled moves (46 pure, 13 edited). Edited cases are
built so that no unchanged run inside the block meets the pure minimum size,
so the only sensible answer is one edited block; 041 is the exception that
tests joining such pieces. 064 and 065 are the same two-line move with 19 and
20 alphanumeric characters, on each side of the minimum size. 067 is a
labeled case the engine gets only partly right: a section that moved with
its last value changed is one edited move to a reader, but joining (moves
spec 4.3 step 4) absorbs edited lines only up to the next pure piece, so
`text.moves@1` reports the first four lines as a pure move. It still matches
the label (Jaccard 0.8), with the wrong kind and boundaries.

## Originality

Every case in this corpus is original and written for diffyman. Inputs are
never copied from another product's test suite, and expected results are never
taken from another product's output: `expected.json` comes from this engine and
is checked by a person. The stand-in diff in `check_labels.py` only checks
that labels are consistent; it is never a source of expected output.
