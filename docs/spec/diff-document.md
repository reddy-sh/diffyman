# Diff Document specification

- **Schema version:** `0.2.0` (draft). Changes from `0.1.0` are listed in §15.
- **Status:** Phase 1. Normative for the reference engine and for the golden corpus.
- **Keywords:** MUST, MUST NOT, SHOULD and MAY are used as in RFC 2119.

## 1. Purpose

The Diff Document is diffyman's only result format. The engine, the CLI (`--json`), every binding and the REST API all return it, and consumers render or store it without re-running the diff. Version 0.2.0 defines `kind: "text"`, with hunks and moved blocks. The other blocks are reserved and stay `[]` or `null`.

This file is the single normative definition of the document shape, the options and the warning codes. [Text granularity and moved blocks](text-granularity-and-moves.md) defines how inline spans and moves are computed and refers back here for every shape. Where the two disagree, this file wins.

## 2. Versioning

1. `schemaVersion` is a semver string. During `0.y.z`, a breaking change bumps `y` and an additive change or fix bumps `z`. From `1.0.0` on, a **major** bump may remove or rename fields and needs a migration note. A **minor** bump adds optional fields or enum values. A **patch** bump fixes docs or bugs and leaves the output shape unchanged.
2. **Additive evolution.** Readers MUST ignore unknown fields. Readers MUST treat a known field that is missing as `null`, and writers never leave one out (§4). A typed binding whose field cannot hold `null` reads a missing field as `[]` for arrays, `{}` for objects, `"unknown"` for enums, and otherwise the empty value of its type (`""`, `0`, `false`). A field name is never reused for a different meaning. A deprecated field stays for at least two minor versions.
3. **Open enums.** Every enum may gain values in a minor version. Typed bindings MUST map an unknown value to `"unknown"` when reading. Writers MUST NOT emit `"unknown"`, and an API MUST reject options that hold it (§9).
4. Readers MUST accept any document with the same major version, or the same `0.y` while in 0.x. Readers SHOULD reject documents from a newer major version.

## 3. Conformance

Two documents are **conformant-equal** when their RFC 8785 bytes (§5) are identical once the top-level `provenance` key is removed. An engine or binding conforms when, for every case `tests/corpus/<case>/{left.*, right.*, options.json, expected.json}`, its output is conformant-equal to `expected.json`. That file MAY be pretty-printed and SHOULD leave out `provenance`. The harness passes the corpus file names (such as `left.txt`) as the input names. A document with an `approximate` warning is never a valid expected result. Each algorithm is defined by its reference implementation plus the rules in §7.

## 4. Presence rules: null, absent and empty

- Every field this spec defines MUST be present, at every nesting level. Writers never leave a field out.
- `null` means "not applicable" or "unknown". The object blocks `table` and `tree` are `null` when unused.
- `[]` means "nothing to report". The array blocks (`hunks`, `moves`, `structural`, `regions`, `bytes`, `warnings`) are never `null`.
- Writers MUST NOT add fields outside `extensions` and `provenance`.

## 5. Canonical serialization and `id`

The canonical form is **RFC 8785 JSON Canonicalization Scheme (JCS)**:

- UTF-8 output with no BOM and no insignificant whitespace.
- Object keys sorted by their UTF-16 code units. Key order in pretty-printed output does not matter.
- Strings escaped the JCS way: only `"`, `\` and U+0000 to U+001F are escaped. `\b \t \n \f \r` are used where they apply, and `\u00xx` with lowercase hex for the rest. All other characters are written literally.
- Numbers serialized like ECMAScript `Number.prototype.toString`: `1`, not `1.0`; `0.7273`; never an exponent for the values this spec allows. NaN and Infinity are forbidden. Integers MUST stay within ±(2^53 − 1).

**`id`** is `"sha256:"` followed by the lowercase hex SHA-256 of the JCS bytes of the document with the top-level `id` and `provenance` keys removed. The same inputs, names and options give the same `id` on every platform and in every binding.

## 6. Top-level fields

| Field | Type | 0.2.0, kind `text` |
|---|---|---|
| `schemaVersion` | string | **Required**: `"0.2.0"` |
| `id` | string | **Required** (§5) |
| `kind` | enum: `text` `code` `structured` `table` `document` `image` `binary` `tree` | **Required**: `"text"` |
| `status` | enum: `identical` `different` `partial` `error` | **Required**: one of `identical`, `different`, `error`. `partial` is reserved. |
| `left`, `right` | File meta (§6.1) | **Required** |
| `positionEncoding` | enum: `utf-8` `utf-16` `codepoint` | **Required**, equal to `options.positionEncoding` |
| `stats` | Stats (§6.3) or `null` | **Required**; `null` only when `status` is `error` |
| `hunks` | Hunk[] (§6.4) | **Required** |
| `moves` | Move[] (§6.5) | **Required**; `[]` unless `options.detectMoves` is `true` |
| `structural` | Change[] | `[]` (reserved) |
| `table` | object or `null` | `null` (reserved) |
| `regions` | Region[] | `[]` (reserved) |
| `bytes` | ByteRange[] | `[]` (reserved) |
| `tree` | object or `null` | `null` (reserved) |
| `warnings` | Warning[] (§10) | **Required** |
| `options` | Options (§9), fully resolved | **Required** |
| `provenance` | object (§11) | **Required**; left out of `id` and conformance |
| `extensions` | object | `{}` in 0.2.0 |

**Status rules.** `error` if `warnings` contains any entry with `severity: "error"`. Otherwise `identical` if there are no change ops (§7.3) under the resolved options. Otherwise `different`. `ignored` ops (§6.4) are not change ops. So two files that differ only in ignored whitespace, only in a BOM, only in text masked by `ignore.regex`, or (with `ignore.blankLines`) only in blank lines, are `identical`, and their `hunks` is `[]` (§7.4). Compare `left.sha256` with `right.sha256` to test byte equality.

When `status` is `error`: `hunks` and `moves` are `[]` and `stats` is `null`. File meta and per-side warnings are still filled in for every side that decodes (§8).

### 6.1 File meta (`left`, `right`)

| Field | Type | Rule |
|---|---|---|
| `name` | string or `null` | The final path component given by the caller, or `null` when the input is bytes with no name. Never a full path. |
| `mediaType` | string or `null` | `"text/plain"` for kind `text` in 0.2.0 |
| `size` | integer | Length of the raw input in bytes, including any BOM |
| `sha256` | string | Lowercase hex SHA-256 of the raw input bytes |
| `encoding` | string or `null` | `"utf-8"` when the input decodes, `null` when it does not |
| `bom` | boolean | `true` if the raw input starts with EF BB BF |
| `lineTerminator` | enum: `lf` `crlf` `cr` `mixed` `none`, or `null` | One value when every terminated line uses that style. `mixed` when more than one style appears. `none` when no line has a terminator. `null` if the input does not decode. |
| `finalNewline` | boolean or `null` | `true` if the last line has a terminator. `false` when it has none or the file has no lines. `null` if the input does not decode. |
| `lineCount` | integer or `null` | Number of lines (§7.2). `null` if the input does not decode. |

### 6.2 Ranges and positions

- A **line range** is `{start, count}`. `start` is 1-based and the range is half-open: it covers the lines `start` to `start + count − 1`.
- An **empty range** (`count: 0`) marks an insertion point just before line `start`. Appending after the last line of an N-line file uses `start: N + 1`. An empty file uses `{start: 1, count: 0}`.
- **Inline offsets** count from the start of the line's content. The terminator is not content, and a BOM is never counted. The unit depends on `positionEncoding`: `utf-8` counts bytes (the default), `utf-16` counts UTF-16 code units, and `codepoint` counts Unicode scalar values. Inline ranges are half-open `[start, end)`.

### 6.3 Stats

All fields are integers except `similarity`. The counts cover the whole file, not only the lines shown in hunks.

| Field | Definition |
|---|---|
| `linesLeft`, `linesRight` | `lineCount` of each side (L, R) |
| `linesAdded` | Sum of `rightRange.count` over `insert` and `replace` ops |
| `linesRemoved` | Sum of `leftRange.count` over `delete` and `replace` ops |
| `linesUnchanged` | Number of lines covered by `equal` ops in the full op list (§7.3) (U) |
| `changeCount` | Number of change ops |
| `hunkCount` | Length of `hunks` |
| `similarity` | 2U / (L′ + R′), rounded half up to 4 decimal places; `1` when L′ + R′ = 0 |

Sums run over the full op list (§7.3), not only over `hunks`. Lines of `ignored` ops count in none of `linesAdded`, `linesRemoved` and `linesUnchanged`. With IL and IR the left and right lines of the `ignored` ops, L′ = L − IL and R′ = R − IR, so `linesUnchanged + linesRemoved + IL = L` and `linesUnchanged + linesAdded + IR = R`. Without `ignore.blankLines` there are no `ignored` ops, and L′ = L, R′ = R. Leaving ignored lines out of the denominator keeps `similarity` at `1` whenever `status` is `identical`.

Compute it with integers: with D = L′ + R′, `n = floor((40000·U + D) / (2·D))`, and `similarity` is the double nearest to `n / 10000`. "4 decimals" is the value's precision, not its text width: JCS writes `0.75` and `1`, never `0.7500`.

### 6.4 Hunks

```
Hunk   = { id, left: Range, right: Range, ops: Op[] }
Op     = { op, leftRange: Range, rightRange: Range, inline: Span[] }
Span   = { side, line, start, end, op }
```

- `id` is `"h1"`, `"h2"`, ... in document order.
- `op` is one of `equal`, `insert`, `delete`, `replace`, `ignored`. `equal` has equal non-zero counts on both sides. `insert` has `leftRange.count = 0`. `delete` has `rightRange.count = 0`. `replace` has non-zero counts on both sides, which may differ. `insert`, `delete` and `replace` are the **change ops**.
- `ignored` (since 0.2.0) appears only with `ignore.blankLines: true`. It covers lines that were a change op but are all blank (§7.3 step 3). Its counts may differ, and either one (not both) may be 0. It is not a change op: it does not make the document `different`, it is not counted in `stats` (§6.3), it never starts a hunk (§7.4) and its lines are never part of a move (§6.5). Renderers SHOULD show it like the change it would have been, for example dimmed. Its `inline` is always `[]`.
- An op always has both ranges. An empty range carries the insertion point (§6.2).
- The ops of a hunk tile the hunk's ranges in order with no gaps: each op starts where the previous one ended, on both sides. `hunk.left.start` is the `leftRange.start` of the first op, and `hunk.left.count` is the sum of the ops' `leftRange.count`. The right side works the same way.
- `inline` is `[]` unless `granularity` is not `line` (§9). When used, spans appear only on `replace` ops. `side` is `"left"` (with `op: "delete"`) or `"right"` (with `op: "insert"`). `line` is the absolute 1-based line number on that side. `start` and `end` are inline offsets (§6.2). Left spans come first, then right spans, each sorted by `(line, start)`.

### 6.5 Moves

```
Move    = { id, kind, from: MoveEnd, to: MoveEnd, similarity, fromOps: OpRef[], toOps: OpRef[] }
MoveEnd = { side, range: Range }
OpRef   = { hunk, op }
```

A move says that a block of deleted left lines reappears as a block of inserted right lines. Which blocks count as moves is defined in [text-granularity-and-moves.md](text-granularity-and-moves.md) §4. This section is the only definition of the output.

- `moves` is `[]` unless `options.detectMoves` is `true`, and also when detection is skipped as a whole (`moves-skipped-limit`, §10). When only the edited-move search hit a limit, `moves` holds the moves found (§10).
- `id` is `"m1"`, `"m2"`, ... in array order.
- `kind` is `pure` (the lines match pairwise under the move key) or `edited` (they match with small edits). It is an open enum (§2.3).
- `from.side` is always `"left"` and `to.side` is always `"right"`. Each `range` is a line range (§6.2) with `count ≥ 1`.
- Every line of `from.range` lies in a `delete` or `replace` op, and every line of `to.range` lies in an `insert` or `replace` op. A line belongs to at most one move. Moves never change `hunks`, `stats` or any op: a moved line stays in its op, and `moves` adds the relation.
- `similarity` is the block similarity, computed with integers in basis points (1 bp = 0.0001) and written as the double nearest to `bp / 10000`, exactly like `stats.similarity` (§6.3, §5): `1` for every pure move, for example `0.875` for an edited one, never `1.0000`.
- `fromOps` and `toOps` list every op the left and right range touch, in document order, each as `{hunk, op}`: `hunk` is the hunk `id` and `op` the 0-based index into that hunk's `ops`. Neither list is empty. Both depend on `options.context`, because context decides how ops are grouped into hunks.
- Entries are sorted by `from.range.start`, then `to.range.start`.
- Derived data is not stored. Renderers compute zebra stripes themselves, and the number of moved left lines is the sum of `from.range.count`. There is no move field in `stats`.
- 0.2.0 has no per-line pairing for edited moves. A later minor version may add one as a new field.

### 6.6 Reserved blocks (shapes are provisional; none is emitted in 0.2.0)

| Block | Planned shape |
|---|---|
| `structural[]` | `{path, op: add\|remove\|change\|move\|rename\|typeChange, leftValue, rightValue, nodeType}`; `path` is a JSON Pointer for JSON/YAML/TOML |
| `table` | `{sheets: [{name, op, rowMap[], colMap[], cells: [{ref, row, col, op, left: {value, formula, type}, right}]}]}` |
| `regions[]` | `{page, bbox: [x0, y0, x1, y1], unit: "pt"\|"px", side, op, textRef, hunkRef}` |
| `bytes[]` | `{leftOffset, leftLength, rightOffset, rightLength, op}` |
| `tree` | `{entries: [{path, status: added\|removed\|modified\|unchanged\|moved\|typeChange, size, sha256, childDocument\|childRef}]}` |
| `extensions` | Keys MUST match `x-<vendor>[-<name>]` (lowercase ASCII, digits and `-`). The values belong to the plugin. The block takes part in `id`. |

## 7. Text diff procedure (kind `text`)

### 7.1 Decoding

1. If the raw input starts with EF BB BF, remove those three bytes, set `bom: true` and emit `bom-stripped` for that side.
2. The rest of the input MUST be valid UTF-8 (RFC 3629: no overlong forms, no surrogates). If it is not, emit `invalid-utf8` (severity `error`) for that side. 0.2.0 has **no binary fallback**: the result is `status: "error"`, and a later version may add `kind: "binary"`. Inputs with a UTF-16 or UTF-32 BOM fail this check.
3. The text is not normalized. NFC and NFD forms of the same text compare as different.

### 7.2 Lines and comparison keys

A line is a maximal run of characters that ends with a **terminator** (LF, CRLF or a lone CR) or with the end of input. CR followed by LF is always one CRLF terminator. A terminator at the very end of the input does not start a new line. So `""` has 0 lines, `"a"` has 1 line, `"a\n"` has 1 line, `"\n"` has 1 empty line, and `"a\r\n\r"` has 2 lines.

Each line gets a comparison key. Two lines are equal when their keys are equal:

1. Start with the content (the line without its terminator). In this section, **whitespace** means only U+0009, U+000B, U+000C and U+0020. A line is **blank** when its content is empty or whitespace only.
2. Apply `ignore.regex` (§7.2.1). The result is a sequence of text and placeholders.
3. Apply `ignore.whitespace` to the text. A placeholder is not whitespace: a run of whitespace never spans one, and only text after the last placeholder can be trailing.
   - `none`: the key is the content plus the terminator kind (`lf`, `crlf`, `cr` or `none`). A missing final newline, or a switch from LF to CRLF, counts as a change, as in git.
   - `trailing`: remove trailing whitespace. The terminator is not part of the key.
   - `change`: remove trailing whitespace and replace every other run of whitespace with one U+0020. The terminator is not part of the key. This matches `git diff -b`.
   - `all`: remove all whitespace. The terminator is not part of the key.
4. If `ignore.case` is true, apply Unicode simple case folding (CaseFolding.txt statuses C and S) to the text. The engine records its Unicode version in provenance.

#### 7.2.1 Regex masks

`ignore.regex` is a list of patterns P0, P1, ... (in the order given). Masking hides the parts of a line that are expected to vary, such as timestamps, IDs or durations, while keeping the rest of the line significant.

1. **Dialect.** Patterns use the syntax of the Rust `regex` crate with its default flags: Unicode mode on, case-sensitive, `.` does not match a line terminator. Inline flags such as `(?i)` work. Look-around and backreferences do not exist in this dialect. A pattern that could match invalid UTF-8, such as `(?-u:\xFF)`, is not valid.
2. **Haystack.** Each pattern is matched against the content of one line (no terminator), so `^` and `$` anchor at the start and end of the content. Matching happens before every other key rule, on the original text.
3. **Matches.** For each pattern, the matches are found left to right, leftmost-first and without overlaps, as by the `regex` crate's `find_iter`. Empty matches are dropped: they mask nothing.
4. **Priority.** Patterns are applied in order. A match of Pi is kept unless it overlaps a match kept for an earlier pattern. Partial overlaps are not merged: the later match is dropped whole.
5. **Placeholders.** Each kept match is replaced with the placeholder of its pattern. Placeholders are equal only to placeholders of the same pattern and never to any text.

So with `["\\d+"]`, `id 7` and `id 123` compare equal, but `id 7` and `id ` do not: a masked value is still a value. With `["\\d+", "[a-f]+"]`, `x 12` and `x ab` differ because the two patterns have different placeholders.

Why placeholders and not removal: removing matches would make a line equal to one that lacks the masked text altogether (`2026-03-14T08:00:01Z start` versus `start`), and it would join the text around a match (`a1b` with `\d` removed equals `ab`). A placeholder hides the value but keeps its presence and position.

Limits (all deterministic):

- At most 16 patterns, each at most 1024 Unicode scalar values long.
- The compiled size of each pattern is at most 50000 NFA units: the states plus the transitions (sparse transitions, 256 for a dense state, alternates of a union) of its forward Thompson NFA as built by the pinned `regex-automata` crate (`diffyman-algo` `key::nfa_units`). This count is the same on every platform, unlike the byte sizes the regex compilers use for their own limits. For example `\w{31}` is within the limit and `\w{32}` is not.

A pattern that does not compile is an `invalid-regex` error (§10). Too many patterns, a pattern that is too long or compiles too large is a `limit-exceeded` error. Both give a document with `status: "error"`, its file meta and the echoed options (§9). Matching itself takes time linear in the line length for each pattern, so no pattern can be catastrophic.

Inline refinement (`text.refine@1`) and move keys (`text.moves@1`) do not apply the masks: spans may cover masked text, and moved lines must match without them ([moves spec](text-granularity-and-moves.md) §3.2, §4.4).

### 7.3 Edit script and grouping

1. Run the selected algorithm (§12) over the two key sequences. The result is a sequence of matched and unmatched lines.
2. Build the **full op list**, which tiles both files completely. Each maximal run of matched lines becomes one `equal` op. Each maximal run of unmatched lines between two equal ops (or a file edge) becomes one **change op**: `replace` if it has lines on both sides, `delete` if it has left lines only, and `insert` if it has right lines only. So two change ops are never adjacent, however the algorithm interleaved its deletions and insertions.
3. If `ignore.blankLines` is true, every change op whose lines are all blank (§7.2), on both sides, becomes an `ignored` op with the same ranges. Blankness is judged on the original content: regex masks and terminators do not matter, so `""` with LF versus `"  "` with CRLF is blank on both sides. A change op that holds any non-blank line stays a change op as a whole; it is not split. This matches git's `--ignore-blank-lines`. Like the change op it replaces, an `ignored` op is never adjacent to a change op or to another `ignored` op.
4. No other post-processing (slider or indent heuristics) is applied in algorithm version `@1`.

### 7.4 Hunks

Let `C` be `options.context`. Context is counted in `equal` lines only: `ignored` ops are shown whole where they fall inside a hunk, and their lines never count toward `C`.

- No change ops: `hunks` is `[]`. In particular, `ignored` ops never form a hunk of their own.
- Two consecutive change ops go in the **same hunk** when the ops between them hold at most `2·C` equal lines in total. Those ops (`equal` and `ignored`) are then included whole. Otherwise the change ops go in separate hunks.
- Before its first change op, a hunk takes the ops that precede it, nearest first, until `C` equal lines are taken: an `equal` op contributes at most the lines still needed (its last ones), an `ignored` op is taken whole without using any of `C`, and the walk stops when `C` equal lines are taken or at the file start. After its last change op, the hunk takes the following ops the same way (an `equal` op contributes its first lines). A clipped equal op keeps `op: "equal"` with the clipped ranges. Ops with count 0 are never emitted.
- With `C = 0`, every change op is its own hunk and holds no other op.
- Without `ignored` ops these rules are the 0.1.0 rules: the equal op between two change ops is included whole when its count is at most `2·C`, and a hunk starts and ends with at most `C` equal lines.
- Hunks are ordered by `left.start`, which is also the order of `right.start`.

## 8. Line terminators, BOM and invalid input

| Situation | Effect |
|---|---|
| Non-empty last line without a terminator | `finalNewline: false`, warning `no-newline-at-eof` for that side |
| More than one terminator style in one file | `lineTerminator: "mixed"`, warning `mixed-line-endings` |
| UTF-8 BOM | Removed before splitting into lines, `bom: true`, warning `bom-stripped`. Offsets never count the BOM. |
| Invalid UTF-8 | `invalid-utf8` error for that side. Its `encoding`, `lineTerminator`, `finalNewline` and `lineCount` are `null`, and it gets no other warnings. The other side is still processed. |

## 9. Options

Each corpus case's `options.json`, and the options parameter of every API, is an object with the shape below. Every key is optional on input, including keys inside `ignore`, and a missing key takes its default. The document's `options` field echoes the **fully resolved** object: every key present, defaults filled in, and `regex` in the order the caller gave.

```json
{
  "algorithm": "histogram",
  "granularity": "line",
  "ignore": { "whitespace": "none", "case": false, "blankLines": false, "regex": [] },
  "context": 3,
  "positionEncoding": "utf-8",
  "detectMoves": false
}
```

| Key | Values (default first) | 0.2.0 support |
|---|---|---|
| `algorithm` | `histogram`, `myers`, `patience` | Required |
| `granularity` | `line`, `word`, `char`, `smart` | All required. `line` gives no inline spans. The other values fill `inline` on `replace` ops (§6.4) as defined in [text-granularity-and-moves.md](text-granularity-and-moves.md) §3, algorithm `text.refine@1` (`word`: UAX #29 word segments, `char`: extended grapheme clusters, `smart`: chosen per op). |
| `ignore.whitespace` | `none`, `trailing`, `change`, `all` | Required (§7.2) |
| `ignore.case` | `false`, `true` | Required (§7.2) |
| `ignore.blankLines` | `false`, `true` | Both required (since 0.2.0). `true` turns change ops whose lines are all blank into `ignored` ops (§7.3 step 3). |
| `ignore.regex` | array of strings (default `[]`) | Required (since 0.2.0). Each pattern masks its matches in the comparison key (§7.2.1). The array is echoed in the given order. |
| `context` | integer from 0 to 2^31 − 1 (default 3) | Required (§7.4) |
| `positionEncoding` | `utf-8`, `utf-16`, `codepoint` | All required. It affects only the offsets of `inline` spans (§6.2), so with granularity `line` it only shows up in the echoed options. |
| `detectMoves` | `false`, `true` | Both required. `true` fills `moves[]` (§6.5) using the detection of [text-granularity-and-moves.md](text-granularity-and-moves.md) §4, algorithm `text.moves@1` (pure and edited moves). It never changes `hunks` or `stats`; with a `granularity` other than `line`, moved lines get no inline spans. |

- An options object with an unknown key, a wrong type or an unknown enum value is **invalid**. The API rejects it with an error and returns no document. Unknown keys are rejected, not ignored, because ignoring them would silently change the result. Typed APIs apply the same rules to options built in code: an `"unknown"` enum value (§2.3) or an out-of-range `context` is rejected the same way, so a document never echoes `"unknown"`.
- There is no separate option for move kinds or for whitespace in moves. `detectMoves` is the only switch; which move kinds it finds is fixed by the moves algorithm ID (§12), and move keys follow `ignore.whitespace` and `ignore.case` (moves spec §4.4).
- A valid value that this engine version does not support produces a document with `status: "error"` and an `unsupported-option` error. No value is deferred in 0.2.0; the code stays for later options.
- `ignore.regex` must be an array of strings; anything else is invalid. Whether a string compiles is checked when the diff runs: a pattern that does not compile gives an `invalid-regex` error document, and one over a limit of §7.2.1 a `limit-exceeded` error document. Compiling depends on the regex dialect, not on the shape of the options, and the document still echoes the options, so the caller can see what failed.

## 10. Warnings and errors

```
Warning = { code, severity: "warning" | "error", side: "left" | "right" | null, message }
```

- `code` is a stable kebab-case ID. Consumers act on the code, never on the message.
- `message` is fixed English text for each code, exactly as in the table below. This keeps `id` stable. Translations map the code to their own text.
- There is at most one entry per `(code, side)`. Entries are sorted by side (`left`, then `right`, then `null`), then by `code` in code-point order.
- Every entry has exactly these four fields, whatever its code. Entries carry no counts or other per-case data.
- This table is the only list of codes. Other specs refer to it and never define codes of their own.

| Code | Severity | Side | Message (exact) |
|---|---|---|---|
| `bom-stripped` | warning | left/right | `UTF-8 byte order mark removed before comparison.` |
| `mixed-line-endings` | warning | left/right | `File uses more than one line terminator style.` |
| `no-newline-at-eof` | warning | left/right | `No newline at end of file.` |
| `approximate` | warning | null | `Diff search was cut short; the result is valid but may not be minimal.` |
| `refine-skipped-limit` | warning | null | `Inline highlighting was skipped for changes that exceed size limits.` |
| `moves-skipped-limit` | warning | null | `Moved-block detection was skipped because too many lines changed.` |
| `invalid-utf8` | error | left/right | `Input is not valid UTF-8.` |
| `invalid-regex` | error | null | `An ignore.regex pattern is not a valid regular expression.` |
| `unsupported-option` | error | null | `An option value is not supported by this engine version.` |
| `limit-exceeded` | error | null | `Input exceeds a configured size or complexity limit.` |

`approximate` means a cost limit or a host-set deadline stopped the search early. The hunks are still a valid edit script. An engine MAY set it whenever it cannot rule out that a cost limit engaged; the reference engine sets it when more than 500 changed lines have a key that also occurs on the other side (`diffyman-algo` `line_diff::is_approximate`). Wall-clock deadlines are not deterministic, so 0.1.0 engines apply none by default.

`refine-skipped-limit` appears once, with `side: null`, when `granularity` is not `line` and at least one `replace` op got `inline: []` because it exceeds the refinement limits (moves spec §3.1, §3.4). It is not repeated per op and carries no count. Readers that need the affected ops look for `replace` ops with empty `inline`. The `smart` noise cutoff (moves spec §3.3) also leaves `inline` empty, so this is an upper bound, not an exact list.

`moves-skipped-limit` appears when `detectMoves` is `true` and a work limit of the moves spec engaged. With the candidate guard (moves spec §4.3), detection is skipped and `moves` is `[]`. With the limits of the edited-move search (moves spec §4.5), the pure moves and the edited moves found before the limit are kept. Hunks and stats are unaffected either way.

Both are warnings: the document stays valid, and its `status` is `identical` or `different` as usual.

`invalid-regex` (since 0.2.0) appears once, with `side: null`, when at least one `ignore.regex` pattern does not compile (§7.2.1). It does not say which one; the options are echoed. `limit-exceeded` also covers the pattern limits of §7.2.1. Both can appear in one document when different patterns fail for different reasons.

## 11. Provenance

Provenance is audit data. It is left out of `id` and of conformance, and it may differ between runs. Writers MUST include `engine` `{name, version}`, `algorithm` (§12), `adapter` `{id, version}`, `binding` (`rust`, `cli`, `python`, `node`, `wasm`, `java`, `c`), `platform` `{os, arch}` and `timing` `{totalMicros}`. Timing values appear only here. Writers MAY add more keys, such as library versions or the Unicode version. Readers MUST NOT use provenance to interpret the rest of the document.

## 12. Algorithm IDs

An algorithm ID has the form `<kind>.<name>@<n>`. The line diff IDs are `text.histogram@1`, `text.myers@1` and `text.patience@1`, chosen by `options.algorithm` and recorded as `provenance.algorithm`. Inline refinement (`text.refine@1`) and moved-block detection (`text.moves@1`, pure and edited moves) have their own IDs, defined in the moves spec. Writers record them in provenance when they run. Any change that alters the output for any input bumps `n`. That includes a library upgrade, a heuristic, a deterministic cost limit or a grouping rule. The bump is recorded in the changelog, and the corpus expected files are regenerated in the same change. The regex dialect and its Unicode tables come from the pinned `regex` crate, so an upgrade that changes any match, or the NFA unit count of any pattern (§7.2.1), bumps the line diff IDs too. A schema change on its own does not bump algorithm IDs.

## 13. Ordering and determinism

Arrays follow file position (hunks by `left.start`, ops in tiling order), with spans ordered as in §6.4, moves as in §6.5 and warnings as in §10. Hash-map order and locale never matter. The only non-integer numbers are `stats.similarity` and `moves[].similarity`, both basis points written as in §6.3. Outside `provenance`, the output depends only on the input bytes, the input names and the resolved options.

## 14. Worked example

`left.txt` (36 bytes, final LF):

```
apple
banana
cherry
date
elderberry
```

`right.txt` (42 bytes, **no** final newline after `fig`):

```
apple
blueberry
cherry
date
elderberry
fig
```

`options.json`: `{"context": 1}`. Everything else takes its default.

The full op list is: equal L1/R1, replace L2/R2, equal L3–5/R3–5, insert before L6 / R6. The equal op between the two changes has 3 lines, which is more than 2·1, so there are two hunks. U = 4, L = 5, R = 6, and `similarity = floor(160011 / 22) / 10000 = 0.7273`.

The result is pretty-printed below for reading. The canonical form is JCS (§5), and the `id` below is the hash of that form without `id` and `provenance`.

```json
{
  "schemaVersion": "0.2.0",
  "id": "sha256:95ff6bb81d8f60205c99119cc386ddfa9235ee9f83e253fc10fd6f166e604a54",
  "kind": "text",
  "status": "different",
  "left": {
    "name": "left.txt", "mediaType": "text/plain", "size": 36,
    "sha256": "9832a4e44238cd39a2b5561f8cce7a35e2e8d4b86a444f18a1ee260941f7bfdc",
    "encoding": "utf-8", "bom": false, "lineTerminator": "lf",
    "finalNewline": true, "lineCount": 5
  },
  "right": {
    "name": "right.txt", "mediaType": "text/plain", "size": 42,
    "sha256": "2ae70b2ff240a155ac5950ab03bdd1a37b2195c15916cf338ce8bf600254095d",
    "encoding": "utf-8", "bom": false, "lineTerminator": "lf",
    "finalNewline": false, "lineCount": 6
  },
  "positionEncoding": "utf-8",
  "stats": {
    "linesLeft": 5, "linesRight": 6, "linesAdded": 2, "linesRemoved": 1,
    "linesUnchanged": 4, "changeCount": 2, "hunkCount": 2, "similarity": 0.7273
  },
  "hunks": [
    {
      "id": "h1",
      "left": { "start": 1, "count": 3 },
      "right": { "start": 1, "count": 3 },
      "ops": [
        { "op": "equal",   "leftRange": { "start": 1, "count": 1 }, "rightRange": { "start": 1, "count": 1 }, "inline": [] },
        { "op": "replace", "leftRange": { "start": 2, "count": 1 }, "rightRange": { "start": 2, "count": 1 }, "inline": [] },
        { "op": "equal",   "leftRange": { "start": 3, "count": 1 }, "rightRange": { "start": 3, "count": 1 }, "inline": [] }
      ]
    },
    {
      "id": "h2",
      "left": { "start": 5, "count": 1 },
      "right": { "start": 5, "count": 2 },
      "ops": [
        { "op": "equal",  "leftRange": { "start": 5, "count": 1 }, "rightRange": { "start": 5, "count": 1 }, "inline": [] },
        { "op": "insert", "leftRange": { "start": 6, "count": 0 }, "rightRange": { "start": 6, "count": 1 }, "inline": [] }
      ]
    }
  ],
  "moves": [],
  "structural": [],
  "table": null,
  "regions": [],
  "bytes": [],
  "tree": null,
  "warnings": [
    { "code": "no-newline-at-eof", "severity": "warning", "side": "right", "message": "No newline at end of file." }
  ],
  "options": {
    "algorithm": "histogram",
    "granularity": "line",
    "ignore": { "whitespace": "none", "case": false, "blankLines": false, "regex": [] },
    "context": 1,
    "positionEncoding": "utf-8",
    "detectMoves": false
  },
  "provenance": {
    "engine": { "name": "diffyman", "version": "0.1.0" },
    "algorithm": "text.histogram@1",
    "adapter": { "id": "text", "version": "0.1.0" },
    "binding": "rust",
    "platform": { "os": "macos", "arch": "aarch64" },
    "timing": { "totalMicros": 412 }
  },
  "extensions": {}
}
```

Line 5 is `equal` because it ends with LF in both files. If `left.txt` had also ended in `fig` with a final LF, the two `fig` lines would differ under `whitespace: "none"` and match under any other mode.

## 15. Changes from 0.1.0

Every change is additive in shape: no field is removed or renamed, and two enum values are new.

- `ignore.blankLines: true` and a non-empty `ignore.regex` are supported (§7.2.1, §7.3 step 3) instead of giving an `unsupported-option` error document.
- New op kind `ignored` (§6.4), with its effect on status (§6), stats and `similarity` (§6.3) and hunks (§7.4).
- New warning code `invalid-regex` (§10). `limit-exceeded` also covers the pattern limits.
- For options that 0.1.0 supported, the output is unchanged apart from `schemaVersion` and therefore `id`.

The version bumps `y` (§2.1) although the shape only grows: the same options now give a different result (a diff instead of an error document), and an `ignored` op breaks the 0.1.0 rule that every op other than `equal` is a change. A 0.1 reader rejects 0.2 documents (§2.4) instead of misreading them.
