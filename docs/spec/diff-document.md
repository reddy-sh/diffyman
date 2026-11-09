# Diff Document specification

- **Schema version:** `0.4.0` (draft). Changes from `0.3.0` are listed in §20, changes from `0.2.0` in §18, changes from `0.1.0` in §15.
- **Status:** Phase 1. Normative for the reference engine and for the golden corpus.
- **Keywords:** MUST, MUST NOT, SHOULD and MAY are used as in RFC 2119.

## 1. Purpose

The Diff Document is diffyman's only result format. The engine, the CLI (`--json`), every binding and the REST API all return it, and consumers render or store it without re-running the diff. Version 0.4.0 defines `kind: "text"`, with hunks and moved blocks, `kind: "binary"`, with changed byte ranges, and `kind: "tree"`, with the entries of two folders or archives and a child document for each changed file. Type detection (§16) decides which one a pair of inputs gets. The other blocks are reserved and stay `[]` or `null`.

This file is the single normative definition of the document shape, the options and the warning codes. [Text granularity and moved blocks](text-granularity-and-moves.md) defines how inline spans and moves are computed and refers back here for every shape. Where the two disagree, this file wins.

## 2. Versioning

1. `schemaVersion` is a semver string. During `0.y.z`, a breaking change bumps `y` and an additive change or fix bumps `z`. From `1.0.0` on, a **major** bump may remove or rename fields and needs a migration note. A **minor** bump adds optional fields or enum values. A **patch** bump fixes docs or bugs and leaves the output shape unchanged.
2. **Additive evolution.** Readers MUST ignore unknown fields. Readers MUST treat a known field that is missing as `null`, and writers never leave one out (§4). A typed binding whose field cannot hold `null` reads a missing field as `[]` for arrays, `{}` for objects, `"unknown"` for enums, and otherwise the empty value of its type (`""`, `0`, `false`). A field name is never reused for a different meaning. A deprecated field stays for at least two minor versions.
3. **Open enums.** Every enum may gain values in a minor version. Typed bindings MUST map an unknown value to `"unknown"` when reading. Writers MUST NOT emit `"unknown"`, and an API MUST reject options that hold it (§9).
4. Readers MUST accept any document with the same major version, or the same `0.y` while in 0.x. Readers SHOULD reject documents from a newer major version.

## 3. Conformance

Two documents are **conformant-equal** when their RFC 8785 bytes (§5) are identical once the top-level `provenance` key is removed. An engine or binding conforms when, for every case `tests/corpus/<case>/{left.*, right.*, options.json, expected.json}`, its output is conformant-equal to `expected.json`. That file MAY be pretty-printed and SHOULD leave out `provenance`. The harness passes the corpus file names (such as `left.txt`) as the input names. A document with an `approximate` warning is never a valid expected result. Each algorithm is defined by its reference implementation plus the rules in §7. A build without archive support (§19.7) conforms when it gives conformant-equal documents for every other case and refuses the cases §19.7 lists with an `unsupported` error.

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
- The same bound applies to every number written without an exponent. ECMAScript prints a floating-point value of magnitude 2^53 up to (not including) 10^21 as a plain integer literal (`1e19` as `10000000000000000000`), which a reader takes for an integer outside the allowed range, so the text would not canonicalize a second time. Writers MUST reject such a value, as they reject an out-of-range integer. Values from 10^21 up print with an exponent (`1e+21`) and are allowed. This matters only for free-form data (`extensions`, `provenance`, reserved blocks): every number this spec defines is a safe integer or a ratio in `[0, 1]`. (Since 0.3.0.)

**`id`** is `"sha256:"` followed by the lowercase hex SHA-256 of the JCS bytes of the document with the top-level `id` and `provenance` keys removed. The same inputs, names and options give the same `id` on every platform and in every binding.

## 6. Top-level fields

| Field | Type | 0.4.0, kinds `text`, `binary` and `tree` |
|---|---|---|
| `schemaVersion` | string | **Required**: `"0.4.0"` |
| `id` | string | **Required** (§5) |
| `kind` | enum: `text` `code` `structured` `table` `document` `image` `binary` `tree` | **Required**: `"text"`, `"binary"` or `"tree"`, chosen as in §16.2 |
| `status` | enum: `identical` `different` `partial` `error` | **Required**. `partial` appears only for kind `tree` (§19.6). |
| `left`, `right` | File meta (§6.1) | **Required** |
| `positionEncoding` | enum: `utf-8` `utf-16` `codepoint` | **Required**, equal to `options.positionEncoding` |
| `stats` | Stats (§6.3) or `null` | **Required**; `null` only when `status` is `error` |
| `hunks` | Hunk[] (§6.4) | **Required**; `[]` unless kind is `text` |
| `moves` | Move[] (§6.5) | **Required**; `[]` unless `options.detectMoves` is `true` and kind is `text` |
| `structural` | Change[] | `[]` (reserved) |
| `table` | object or `null` | `null` (reserved) |
| `regions` | Region[] | `[]` (reserved) |
| `bytes` | ByteRange[] (§6.7) | **Required**; `[]` unless kind is `binary` |
| `tree` | Tree (§6.8) or `null` | **Required**; `null` unless kind is `tree` (since 0.3.0) |
| `warnings` | Warning[] (§10) | **Required** |
| `options` | Options (§9), fully resolved | **Required** |
| `provenance` | object (§11) | **Required**; left out of `id` and conformance |
| `extensions` | object | `{}` in 0.4.0 |

**Status rules.** `error` if `warnings` contains any entry with `severity: "error"`. Otherwise, for kind `tree`, `partial` if some entry content was not read (§19.6). Otherwise `identical` if there are no change ops (§7.3) under the resolved options (kind `text`), `bytes` is `[]` (kind `binary`), or every entry is `unchanged` (kind `tree`). Otherwise `different`. `ignored` ops (§6.4) are not change ops. So two files that differ only in ignored whitespace, only in a BOM, only in text masked by `ignore.regex`, or (with `ignore.blankLines`) only in blank lines, are `identical`, and their `hunks` is `[]` (§7.4). Compare `left.sha256` with `right.sha256` to test byte equality.

When `status` is `error`: `hunks`, `moves` and `bytes` are `[]`, and `stats` and `tree` are `null`. File meta and per-side warnings are still filled in for every side that decodes (§8).

### 6.1 File meta (`left`, `right`)

| Field | Type | Rule |
|---|---|---|
| `name` | string or `null` | The final path component given by the caller, or `null` when the input is bytes with no name. Never a full path. |
| `mediaType` | string or `null` | The detected media type (§16.1), such as `"text/plain"`, `"text/csv"` or `"image/png"`, whatever the kind. Writers since 0.3.0 never write `null`. |
| `size` | integer | Length of the raw input in bytes, including any BOM |
| `sha256` | string | Lowercase hex SHA-256 of the raw input bytes |
| `encoding` | string or `null` | Kind `text`: `"utf-8"` when the input decodes, `null` when it does not. Kind `binary`: the detected encoding of a text or empty input (`"utf-8"`, `"utf-16le"`, `"utf-16be"`, §16.1), `null` for a binary input. |
| `bom` | boolean | `true` if the raw input starts with EF BB BF |
| `lineTerminator` | enum: `lf` `crlf` `cr` `mixed` `none`, or `null` | One value when every terminated line uses that style. `mixed` when more than one style appears. `none` when no line has a terminator. `null` if the input does not decode, and always `null` for kind `binary`. |
| `finalNewline` | boolean or `null` | `true` if the last line has a terminator. `false` when it has none or the file has no lines. `null` if the input does not decode, and always `null` for kind `binary`. |
| `lineCount` | integer or `null` | Number of lines (§7.2). `null` if the input does not decode, and always `null` for kind `binary`. |

For kind `tree`, `encoding`, `lineTerminator`, `finalNewline` and `lineCount` are `null`. An archive side keeps the meaning above (`size` and `sha256` of the archive bytes, `mediaType` detected). A folder side (§19.1) has `mediaType: "inode/directory"`, `bom: false`, `size` equal to the sum of the `size` of its file entries, and as `sha256` the **manifest digest**: the SHA-256 of the canonical JSON (§5) of the array that holds, for each entry of that side in path order, the array `[path, type, sha256 or target or null]` (§6.8). Two folders with the same entries, types and contents have the same manifest digest. When the side cannot be listed (`invalid-archive`, `limit-exceeded`), a folder side has `size: 0` and the manifest digest of `[]`.

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
| `bytes` | Byte stats (§6.3.1) for kind `binary`; `null` otherwise (since 0.3.0) |
| `tree` | Tree stats (§6.3.2) for kind `tree`; `null` otherwise (since 0.3.0) |

Sums run over the full op list (§7.3), not only over `hunks`. Lines of `ignored` ops count in none of `linesAdded`, `linesRemoved` and `linesUnchanged`. With IL and IR the left and right lines of the `ignored` ops, L′ = L − IL and R′ = R − IR, so `linesUnchanged + linesRemoved + IL = L` and `linesUnchanged + linesAdded + IR = R`. Without `ignore.blankLines` there are no `ignored` ops, and L′ = L, R′ = R. Leaving ignored lines out of the denominator keeps `similarity` at `1` whenever `status` is `identical`.

Compute it with integers: with D = L′ + R′, `n = floor((40000·U + D) / (2·D))`, and `similarity` is the double nearest to `n / 10000`. "4 decimals" is the value's precision, not its text width: JCS writes `0.75` and `1`, never `0.7500`.

#### 6.3.1 Binary stats

For kind `binary`, `stats` counts bytes. A binary document has no lines and no hunks, so `linesLeft`, `linesRight`, `linesAdded`, `linesRemoved`, `linesUnchanged` and `hunkCount` are `0`. `changeCount` is the length of `bytes[]`, and `bytes` is:

| Field | Definition |
|---|---|
| `left`, `right` | `size` of each side (L, R) |
| `added` | Sum of `rightLength` over `bytes[]` |
| `removed` | Sum of `leftLength` over `bytes[]` |
| `unchanged` | Bytes outside every range (U): `left − removed`, which equals `right − added` |

`similarity` is 2U / (L + R), computed with integers exactly as above (with D = L + R), and `1` when both inputs are empty.

#### 6.3.2 Tree stats

For kind `tree`, `stats` counts entries (§6.8). `linesLeft`, `linesRight`, `linesAdded`, `linesRemoved`, `linesUnchanged` and `hunkCount` are `0`, `bytes` is `null`, and `changeCount` is the number of entries whose `status` is not `unchanged`. `tree` is:

| Field | Definition |
|---|---|
| `entriesLeft`, `entriesRight` | Number of entries listed on each side (L, R): entries with a non-null `left`, and with a non-null `right` |
| `added`, `removed`, `modified`, `moved`, `typeChanged`, `unchanged` | Number of entries with that `status` (`typeChanged` counts `typeChange`); `unchanged` is U |

`similarity` is 2U / (L + R), computed with integers exactly as above (with D = L + R), and `1` when both sides have no entries. A `moved` entry counts on both sides but not in U. Child documents are not counted.

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
- `inline` is `[]` unless `granularity` is not `line` (§9). When used, spans appear only on `replace` ops, and never on a line that belongs to a move: an edited move carries the spans of its lines itself (`moves[].inline`, §6.5, since 0.4.0). `side` is `"left"` (with `op: "delete"`) or `"right"` (with `op: "insert"`). `line` is the absolute 1-based line number on that side. `start` and `end` are inline offsets (§6.2). Left spans come first, then right spans, each sorted by `(line, start)`.

### 6.5 Moves

```
Move    = { id, kind, from: MoveEnd, to: MoveEnd, similarity, fromOps: OpRef[], toOps: OpRef[], inline: Span[] }
MoveEnd = { side, range: Range }
OpRef   = { hunk, op }
```

A move says that a block of deleted left lines reappears as a block of inserted right lines. Which blocks count as moves is defined in [text-granularity-and-moves.md](text-granularity-and-moves.md) §4. This section is the only definition of the output.

- `moves` is `[]` unless `options.detectMoves` is `true`, and also when detection is skipped as a whole (`moves-skipped-limit`, §10). When only the edited-move search hit a limit, `moves` holds the moves found (§10).
- `id` is `"m1"`, `"m2"`, ... in array order.
- `kind` is `pure` (the lines match pairwise under the move key) or `edited` (they match with small edits). It is an open enum (§2.3).
- `from.side` is always `"left"` and `to.side` is always `"right"`. Each `range` is a line range (§6.2) with `count ≥ 1`.
- Every line of `from.range` lies in a `delete` or `replace` op, and every line of `to.range` lies in an `insert` or `replace` op. A line belongs to at most one move. Moves never change `hunks`, `stats` or any op: a moved line stays in its op, and `moves` adds the relation.
- `similarity` is the block similarity, computed with integers in basis points (1 bp = 0.0001) and written as the double nearest to `bp / 10000`, exactly like `stats.similarity` (§6.3, §5): `1` for every pure move, for example `0.875` for an edited one, never `1.0000`. An edited move can also have `1` when all its non-trivial lines pair with equal move keys but it is not pure, for example because its indentation delta changes or one side has an extra blank line ([moves spec](text-granularity-and-moves.md) §4.5); `kind` tells the two apart.
- `fromOps` and `toOps` list every op the left and right range touch, in document order, each as `{hunk, op}`: `hunk` is the hunk `id` and `op` the 0-based index into that hunk's `ops`. Neither list is empty. Both depend on `options.context`, because context decides how ops are grouped into hunks.
- Entries are sorted by `from.range.start`, then `to.range.start`.
- `inline` (since 0.4.0) holds what changed inside an edited move, so a reader sees the renamed variable or the fixed typo of a block that also moved. It has the `Span` shape and order of an op's `inline` (§6.4): `side` `"left"` with `op: "delete"` for lines of `from.range`, `side` `"right"` with `op: "insert"` for lines of `to.range`, `line` the absolute 1-based line number, `start` and `end` offsets in `positionEncoding` units (§6.2), left spans first, then right spans, each sorted by `(line, start)`. Every span lies inside the move's own range on its side. The spans are those of `text.refine@1` for one `replace` op made of the two ranges, with the same `granularity`, ignore rules, regex masks, limits and `smart` noise cutoff, except that each line's indentation is left out ([moves spec](text-granularity-and-moves.md) §4.7). `inline` is `[]` when `granularity` is `line`, for every `pure` move, when the move exceeds the refinement limits (then the document has `refine-skipped-limit`, §10) and when the `smart` noise cutoff applies.
- The spans of a moved line are only in its move: the ops never carry spans for lines of a move (§6.4). A renderer shows a moved line with `moves[].inline` of its move and every other changed line with the `inline` of its op.
- Derived data is not stored. Renderers compute zebra stripes themselves, and the number of moved left lines is the sum of `from.range.count`. There is no move field in `stats`.
- There is no per-line pairing of the lines of an edited move; `inline` marks what changed between the two ranges as a whole. A later minor version may add a pairing as a new field.

### 6.6 Reserved blocks (shapes are provisional; none is emitted in 0.4.0)

| Block | Planned shape |
|---|---|
| `structural[]` | `{path, op: add\|remove\|change\|move\|rename\|typeChange, leftValue, rightValue, nodeType}`; `path` is a JSON Pointer for JSON/YAML/TOML |
| `table` | `{sheets: [{name, op, rowMap[], colMap[], cells: [{ref, row, col, op, left: {value, formula, type}, right}]}]}` |
| `regions[]` | `{page, bbox: [x0, y0, x1, y1], unit: "pt"\|"px", side, op, textRef, hunkRef}` |
| `extensions` | Keys MUST match `x-<vendor>[-<name>]` (lowercase ASCII, digits and `-`). The values belong to the plugin. The block takes part in `id`. |

`tree` was reserved up to 0.2.0 and is defined in §6.8 since 0.3.0.

### 6.7 Byte ranges (`bytes[]`, since 0.3.0)

```
ByteRange = { leftOffset, leftLength, rightOffset, rightLength, op }
```

`bytes` lists the changed byte ranges of a binary document (§17). It is `[]` for kind `text`.

- Offsets are 0-based byte positions in the raw input (a BOM is a byte like any other), and lengths count bytes. A range covers `[leftOffset, leftOffset + leftLength)` on the left and `[rightOffset, rightOffset + rightLength)` on the right.
- `op` is `insert` (`leftLength` is 0; `leftOffset` is the insertion point), `delete` (`rightLength` is 0; `rightOffset` is the insertion point) or `replace` (both lengths are non-zero, and they may differ). `equal` and `ignored` never appear.
- Only changes are listed. Entries are sorted by `leftOffset`, which is also the order of `rightOffset`. The bytes between two consecutive ranges, before the first and after the last are equal on both sides and have the same length there: for consecutive ranges a and b, `b.leftOffset − (a.leftOffset + a.leftLength) = b.rightOffset − (a.rightOffset + a.rightLength)`, and that gap is at least 8 bytes (§17 step 4). So the ranges are a complete edit script: copying the gaps from the left and the ranges from the right rebuilds the right input.
- The ranges are valid, not necessarily minimal. Kind `binary` never carries `approximate`; `bytes-coarse-limit` (§10) says when a size limit made a range coarser than byte level.

### 6.8 Tree (`tree`, since 0.3.0)

```
Tree      = { entries: Entry[] }
Entry     = { path, fromPath, status, left: EntrySide | null, right: EntrySide | null,
              childDocument: ChildDocument | null, childRef }
EntrySide = { type, size, sha256, target }
```

`tree` lists the entries of the two sides of a kind `tree` document (§19), one per path. It is `null` for the other kinds and for error documents.

- `path` is the entry's path relative to its side's root: components joined with `/`, no leading `./`, no trailing `/`, never empty (§19.2). For a `moved` entry it is the right path. Entries are sorted by `path`, compared as UTF-8 bytes, and every `path` appears once.
- `fromPath` is the left path of a `moved` entry and `null` for every other status.
- `status` is one of (open enum, §2.3):
  - `added`: only on the right (`left` is `null`). `removed`: only on the left (`right` is `null`).
  - `unchanged`: on both sides with the same type and equal content (§19.4). `modified`: on both sides with the same type and different content. `typeChange`: on both sides with different types.
  - `moved`: a left-only file and a right-only file with the same content, paired by §19.4.
- `left` and `right` describe the entry on each side, or are `null` where it is absent:
  - `type` is `file`, `directory`, `symlink`, `hardlink` or `other` (devices, pipes, sockets and tar entry types with no content; open enum).
  - `size` is the length of a file's content in bytes, as read; for a file whose content was not read (§19.6) it is the size the archive declares, or `null` if unknown. `null` for every other type.
  - `sha256` is the lowercase hex SHA-256 of a file's content, or `null` for every other type and for a file whose content was not read.
  - `target` is the link text of a `symlink` or `hardlink`, exactly as stored and never resolved; `null` for every other type.
- `childDocument` is the Diff Document (§6) of the two contents of a `modified` file entry, with every field except `provenance` (§19.5). It is `null` for every other entry and when the child was left out (`children-omitted`, §10). Its `id` is computed as usual (§5); its `left.name` and `right.name` are the last path components.
- `childRef` is reserved for services that store child documents apart from their parent. It is always `null` in 0.4.0.

## 7. Text diff procedure (kind `text`)

### 7.1 Decoding

1. If the raw input starts with EF BB BF, remove those three bytes, set `bom: true` and emit `bom-stripped` for that side.
2. The rest of the input MUST be valid UTF-8 (RFC 3629: no overlong forms, no surrogates). If it is not, emit `invalid-utf8` (severity `error`) for that side, and the result is `status: "error"`. Inputs with a UTF-16 or UTF-32 BOM fail this check. This only happens with `adapter: "text"`: under `auto` (the default), a pair with an input that is not UTF-8 text never reaches the text adapter and is compared as bytes instead (§16.2).
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

Inline refinement (`text.refine@1`) and move keys (`text.moves@1`) apply the same masks ([moves spec](text-granularity-and-moves.md) §3.2, §4.4). In refinement each kept match is one token that equals any match of the same pattern, so a masked value that changed gets no inline span, and a match with no counterpart is covered by one span. In move keys a kept match is its placeholder, so lines that are equal for the line diff because of a mask are also equal as move keys.

### 7.3 Edit script and grouping

1. Run the selected algorithm (§12) over the two key sequences. The result is a sequence of matched and unmatched lines, which slider compaction (§7.3.1) then puts in canonical positions, for every algorithm.
2. Build the **full op list**, which tiles both files completely. Each maximal run of matched lines becomes one `equal` op. Each maximal run of unmatched lines between two equal ops (or a file edge) becomes one **change op**: `replace` if it has lines on both sides, `delete` if it has left lines only, and `insert` if it has right lines only. So two change ops are never adjacent, however the algorithm interleaved its deletions and insertions.
3. If `ignore.blankLines` is true, every change op whose lines are all blank (§7.2), on both sides, becomes an `ignored` op with the same ranges. Blankness is judged on the original content: regex masks and terminators do not matter, so `""` with LF versus `"  "` with CRLF is blank on both sides. A change op that holds any non-blank line stays a change op as a whole; it is not split. This matches git's `--ignore-blank-lines`. Like the change op it replaces, an `ignored` op is never adjacent to a change op or to another `ignored` op.
4. No other post-processing is applied in algorithm version `@1`.

#### 7.3.1 Slider compaction

A run of unmatched lines on one side is a slider when the line just above it has the same key as its last line (or the line just below it the same key as its first line): it can shift by one line and still describe the same change, as the blank line or closing brace between an inserted function and its neighbour can go at either end of the insertion. Each side is processed in turn, left first, then right (which sees the left result). For each run, top to bottom:

1. Shift it up as far as it goes, then down as far as it goes. A run that comes to touch another run of the same side joins it, and a run that grew is shifted again, until its size stops changing. Note its highest position and the lowest position at which the other side has unmatched lines between the same two matched lines.
2. If it cannot shift, leave it. Otherwise, if such a position on the other side exists, move it to the lowest one, so that a deletion and an insertion next to each other become one `replace` op. Otherwise move it to the position the **indent heuristic** picks among the lowest position and the ones above it, at most `min(size + 1, 100)` positions up (and not above its highest position).
3. The indent heuristic (git's `--indent-heuristic`, the default since git 2.14, with the same weights) rates the two boundaries a position gives the run, one just above its first line and one just below its last line, and takes the position whose two ratings add up best; ties go to the lower position. **Indentation** is measured on the original content: U+0020 counts 1 column, U+0009 moves to the next multiple of 8, the first other character ends it, and it is capped at 200 (a whitespace-only line that reaches the cap counts as indented 200). A line with no other character is **blank** here. For a boundary just above line `s` (`s` = the line count for the end of the file):
   - `indent`: the indentation of line `s` (none when it is blank or at the end of the file); `blanksAbove`: the blank lines directly above the boundary, and `indentAbove` the indentation of the nearest non-blank line above them (none at the file start); `blanksBelow` and `indentBelow` the same for the lines after line `s`. At most 20 blank lines are counted on either side; at 20, the nearest indentation counts as 0.
   - `penalty`: +1 when nothing is above (`indentAbove` none and `blanksAbove` 0); +21 at the end of the file; with `after = 1 + blanksBelow` when `indent` is none and 0 otherwise, and `blanks = blanksAbove + after`, add `-30·blanks + 6·after`.
   - `effective` = `indent`, or `indentBelow` when `indent` is none; it adds to the position's indent sum (none adds -1). When `effective` and `indentAbove` both exist: more indented than above adds +10 with `blanks > 0` and -4 without; equal adds 0; less indented adds +17 with `blanks > 0`, and without blanks +24 when `indentBelow` is more indented than `effective` and +23 otherwise.
   - Position A is better than B when `60·sign(indentSum(A) - indentSum(B)) + penalty(A) - penalty(B) < 0`.
4. Every shift trades an unmatched line for a matched line with the same key on the same side, so the number of unmatched lines on each side and their keys never change; only where the changes sit does. Without `ignore.blankLines`, `stats` other than `changeCount` and `hunkCount` therefore never change either. With it they can, because compaction runs before step 3 of §7.3 and changes which lines form one change op: a run of blank lines that slides next to, or joins, a change with a non-blank line, or is lined up with one on the other side, becomes part of that change op, so its lines are counted in `stats` and in the `similarity` denominator instead of being `ignored`; the reverse can happen too. git applies `--ignore-blank-lines` after its own compaction in the same way. Example (case `text/043`): left `""`, `""`, `a`, `  x`, `a`, `  x`, `a`, `""`; right `""`, `a`, `}`, `  x`, `a`, `""`. The algorithm deletes left line 2 (blank, so `ignored`) and replaces lines 4-5 with `}`; compaction slides the deletion of lines 4-5 up to lines 3-4, where it joins the blank one, so a `delete` of lines 2-4 and an `insert` of `}` remain, with `linesRemoved: 3` instead of 2 and no `ignored` op, as in git.
5. Work is bounded: each shift, each line comparison (plus one unit per 64 bytes compared outside the trimmed common prefix and suffix) and each line the heuristic reads costs one unit, and the pass stops for good once `max(1000000, 64 × (left lines + right lines))` units are spent, leaving the remaining runs where the algorithm put them. No input of ordinary text comes near this limit, and the count is the same on every platform.

With this step, `text.histogram@1` and `text.myers@1` give the same hunks as `git diff --histogram` and `git diff --diff-algorithm=myers` on typical source edits (the differences that remain come from the algorithms themselves and from VT and FF, which are whitespace in §7.2 but not in git).

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
| Invalid UTF-8 (with `adapter: "text"`) | `invalid-utf8` error for that side. Its `encoding`, `lineTerminator`, `finalNewline` and `lineCount` are `null`, and it gets no other warnings. The other side is still processed. Under `adapter: "auto"` the pair is compared as bytes instead (§16.2). |

These rows apply to kind `text`. Binary and tree documents have none of these warnings: a BOM is compared as bytes, and line terminators are not looked at (a tree's child documents carry their own).

## 9. Options

Each corpus case's `options.json`, and the options parameter of every API, is an object with the shape below. Every key is optional on input, including keys inside `ignore`, and a missing key takes its default. The document's `options` field echoes the **fully resolved** object: every key present, defaults filled in, and `regex` in the order the caller gave.

```json
{
  "algorithm": "histogram",
  "granularity": "line",
  "ignore": { "whitespace": "none", "case": false, "blankLines": false, "regex": [] },
  "context": 3,
  "positionEncoding": "utf-8",
  "detectMoves": false,
  "adapter": "auto",
  "tree": {
    "followSymlinks": false, "ignoreFiles": false, "exclude": [],
    "detectMoves": true, "maxDepth": 3, "childBudget": 16777216
  }
}
```

| Key | Values (default first) | 0.4.0 support |
|---|---|---|
| `algorithm` | `histogram`, `myers`, `patience` | Required |
| `granularity` | `line`, `word`, `char`, `smart` | All required. `line` gives no inline spans. The other values fill `inline` on `replace` ops (§6.4) and on edited moves (§6.5) as defined in [text-granularity-and-moves.md](text-granularity-and-moves.md) §3, algorithm `text.refine@1` (`word`: UAX #29 word segments, `char`: extended grapheme clusters, `smart`: chosen per op). |
| `ignore.whitespace` | `none`, `trailing`, `change`, `all` | Required (§7.2) |
| `ignore.case` | `false`, `true` | Required (§7.2) |
| `ignore.blankLines` | `false`, `true` | Both required (since 0.2.0). `true` turns change ops whose lines are all blank into `ignored` ops (§7.3 step 3). |
| `ignore.regex` | array of strings (default `[]`) | Required (since 0.2.0). Each pattern masks its matches in the comparison key (§7.2.1). The array is echoed in the given order. |
| `context` | integer from 0 to 2^31 − 1 (default 3) | Required (§7.4) |
| `positionEncoding` | `utf-8`, `utf-16`, `codepoint` | All required. It affects only the offsets of `inline` spans (§6.2), so with granularity `line` it only shows up in the echoed options. |
| `detectMoves` | `false`, `true` | Both required. `true` fills `moves[]` (§6.5) using the detection of [text-granularity-and-moves.md](text-granularity-and-moves.md) §4, algorithm `text.moves@1` (pure and edited moves). It never changes `hunks` or `stats`; with a `granularity` other than `line`, moved lines get no inline spans on their ops, and an edited move carries its own in `moves[].inline`. |
| `adapter` | `auto`, `text`, `binary`, `tree` | All required (since 0.3.0). Chooses the kind as in §16.2: `auto` gives kind `tree` when both inputs are archives, kind `text` when both inputs are UTF-8 text or empty, and kind `binary` otherwise; `text` always runs the text adapter (so an input that is not UTF-8 is an `invalid-utf8` error); `binary` always compares bytes; `tree` always reads both inputs as archives (an input that is not one is an `invalid-archive` error). A folder is always compared as a tree. |
| `tree.followSymlinks` | `false`, `true` | Both required (since 0.3.0). Folders only (§19.2): `true` lists what a symbolic link points to instead of the link. Archive links are never followed. |
| `tree.ignoreFiles` | `false`, `true` | Both required (since 0.3.0). Folders only: `true` leaves out what the `.gitignore` and `.ignore` files inside each folder exclude (§19.2). |
| `tree.exclude` | array of strings (default `[]`) | Required (since 0.3.0). Patterns in `.gitignore` syntax, matched against entry paths on both sides of every tree, folders and archives (§19.2). Echoed in the given order. |
| `tree.detectMoves` | `true`, `false` | Both required (since 0.3.0). `true` pairs left-only and right-only files with equal content as `moved` entries (§19.4). Independent of the text option `detectMoves`. |
| `tree.maxDepth` | integer from 0 to 16 (default 3) | Required (since 0.3.0). How many levels of archives inside a tree are read as trees (§19.5). |
| `tree.childBudget` | integer from 0 to 2^31 − 1 (default 16777216) | Required (since 0.3.0). The total size in bytes of the canonical JSON of the child documents one top-level document may embed (§19.5). `0` embeds none. |

The binary adapter has no options of its own in 0.4.0. For kind `binary`, `algorithm`, `granularity`, `ignore`, `context`, `positionEncoding`, `detectMoves` and `tree` are echoed but have no effect, except that an `ignore.regex` or `tree.exclude` error (§7.2.1, §19.2) still gives an error document: option errors do not depend on the inputs. For kind `text`, `tree` has no effect in the same way. For kind `tree`, the text and binary options apply to the child documents (§19.5).

- An options object with an unknown key, a wrong type or an unknown enum value is **invalid**. The API rejects it with an error and returns no document. Unknown keys are rejected, not ignored, because ignoring them would silently change the result. Options JSON text that gives the same key twice in one object, at any depth, is invalid too (readers differ on which value wins); the reference engine's error message starts with `duplicate key`. Error messages that quote the input (a key, a path) write control and bidi-control characters as `\u{..}` escapes, so printing one never sends raw terminal escape sequences. Typed APIs apply the same rules to options built in code: an `"unknown"` enum value (§2.3) or an out-of-range `context` is rejected the same way, so a document never echoes `"unknown"`.
- There is no separate option for move kinds or for whitespace in moves. `detectMoves` is the only switch; which move kinds it finds is fixed by the moves algorithm ID (§12), and move keys follow `ignore.whitespace` and `ignore.case` (moves spec §4.4).
- A valid value that this engine version does not support produces a document with `status: "error"` and an `unsupported-option` error. No value is deferred in 0.4.0; the code stays for later options.
- `ignore.regex` must be an array of strings; anything else is invalid. Whether a string compiles is checked when the diff runs: a pattern that does not compile gives an `invalid-regex` error document, and one over a limit of §7.2.1 a `limit-exceeded` error document. Compiling depends on the regex dialect, not on the shape of the options, and the document still echoes the options, so the caller can see what failed. `tree.exclude` works the same way, with `invalid-glob` (§19.2).
- `tree` must be an object whose members follow the table; an unknown key, `null`, or `maxDepth` or `childBudget` out of range is invalid.

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
| `moves-skipped-limit` | warning | null | `Moved-block detection was skipped or cut short by work limits.` |
| `binary-fallback` | warning | null | `One input is not UTF-8 text, so both inputs were compared as bytes.` |
| `bytes-coarse-limit` | warning | null | `Some byte ranges are coarser than single bytes because they exceed size limits.` |
| `non-utf8-path` | warning | left/right | `Some entry names are not valid UTF-8; invalid bytes were replaced with U+FFFD.` |
| `unsafe-path` | warning | left/right | `Some entry paths are absolute or climb out of the tree; they are listed as given and never resolved.` |
| `duplicate-path` | warning | left/right | `Some paths appear more than once; one entry per path was kept.` |
| `encrypted-entry` | warning | left/right | `Some archive entries are encrypted; they were compared by their stored bytes.` |
| `entry-unreadable` | warning | left/right | `Some entries could not be read; they were compared without their content.` |
| `tree-limit` | warning | left/right | `Some entries were not read because they exceed folder or archive limits.` |
| `children-omitted` | warning | null | `Some child documents were left out because they exceed size limits.` |
| `depth-limit` | warning | null | `Nested archives deeper than tree.maxDepth were compared as bytes.` |
| `invalid-utf8` | error | left/right | `Input is not valid UTF-8.` |
| `invalid-regex` | error | null | `An ignore.regex pattern is not a valid regular expression.` |
| `invalid-glob` | error | null | `A tree.exclude pattern is not a valid glob pattern.` |
| `invalid-archive` | error | left/right | `Input is not a readable folder or archive.` |
| `unsupported-option` | error | null | `An option value is not supported by this engine version.` |
| `limit-exceeded` | error | null | `Input exceeds a configured size or complexity limit.` |

`approximate` means a cost limit or a host-set deadline stopped the search early. The hunks are still a valid edit script. An engine MAY set it whenever it cannot rule out that a cost limit engaged; the reference engine sets it when more than 500 changed lines have a key that also occurs on the other side (`diffyman-algo` `line_diff::is_approximate`), except for a pure insertion or deletion: when one side is empty once the common prefix and suffix are trimmed, no search runs and the result is exact. Wall-clock deadlines are not deterministic, so 0.1.0 engines apply none by default.

`refine-skipped-limit` appears once, with `side: null`, when `granularity` is not `line` and at least one `replace` op or edited move (§6.5, since 0.4.0) got `inline: []` because it exceeds the refinement limits (moves spec §3.1, §3.4). It is not repeated per op and carries no count. Readers that need the affected ops look for `replace` ops and edited moves with empty `inline`. The `smart` noise cutoff (moves spec §3.3) also leaves `inline` empty, so this is an upper bound, not an exact list.

`moves-skipped-limit` appears when `detectMoves` is `true` and a work limit of the moves spec engaged. With the candidate guard (moves spec §4.3), detection is skipped and `moves` is `[]`. With the limits of the edited-move search and of long-line comparison (moves spec §4.5), detection is cut short: the pure moves and the edited moves found before the limit are kept, so `moves` may hold entries. Hunks and stats are unaffected either way.

`binary-fallback` (since 0.3.0) appears once, with `side: null`, when `adapter` is `auto`, the document is binary, and at least one input was detected as text (§16.1). That covers a text file against a binary one, and text in an encoding the text adapter does not read (UTF-16) against UTF-8. It does not appear when both inputs are binary, or one is empty: comparing bytes is then the expected result, not a fallback. Which side is not UTF-8 text shows in the file meta (`encoding`, `mediaType`).

`bytes-coarse-limit` (since 0.3.0) appears once, with `side: null`, when a limit of §17 reported ranges coarser than byte level: a region compared position by position or reported as one range (step 4, after the region size and work budget of step 2 and the chunk levels of step 3), or the range count of step 6. The ranges are still a complete edit script; only their precision is lower.

The tree warnings (since 0.3.0, kind `tree` only, §19) appear once per side that has the condition:

- `non-utf8-path`: a folder or tar entry name, or a symbolic link target, is not valid UTF-8 (§19.2).
- `unsafe-path`: an entry path is absolute, starts with a drive letter, or has a `..` component (§19.2). The entry is listed with that path; nothing is ever written to or read from it on disk.
- `duplicate-path`: two entries of one side have the same path after normalization, or a non-directory entry has entries below it (§19.2).
- `encrypted-entry`: a zip entry is encrypted, so its content was not read (§19.6).
- `entry-unreadable`: an entry's content could not be read: an I/O error, a zip compression method other than stored and deflate, or content that does not match its declared size or checksum (§19.6).
- `tree-limit`: a content limit of §19.6 stopped reading: a zip entry over the compression-ratio limit or overlapping an earlier entry, a compressed tar stream over the ratio limit, a path over 4096 bytes, or the per-side content budget.

Each of `encrypted-entry`, `entry-unreadable` and `tree-limit` means some content was not read, and makes the status `partial` (§19.6).

- `children-omitted` (side `null`): a `modified` file entry has `childDocument: null` because its content exceeds the child input limits or the content budget, or its child document does not fit `tree.childBudget` (§19.5). It does not appear when `childBudget` is `0`.
- `depth-limit` (side `null`): in a child document whose inputs are both archives but which lies deeper than `tree.maxDepth` (§19.5): the child is kind `binary` instead of `tree`.

All of these are warnings: the document stays valid, and its `status` is `identical`, `different` or (for the tree content warnings) `partial` as stated.

`invalid-regex` (since 0.2.0) appears once, with `side: null`, when at least one `ignore.regex` pattern does not compile (§7.2.1). It does not say which one; the options are echoed. `limit-exceeded` also covers the pattern limits of §7.2.1, the input size limit of the binary adapter (§17) and the tree limits of §19.6. Both can appear in one document when different patterns fail for different reasons.

`invalid-glob` (since 0.3.0) appears once, with `side: null`, when at least one `tree.exclude` pattern does not compile (§19.2). Like `invalid-regex`, it applies whatever the kind. `invalid-archive` (since 0.3.0) appears for each side of a kind `tree` document that is neither a folder nor a readable archive (§19.1, §19.3).

## 11. Provenance

Provenance is audit data. It is left out of `id` and of conformance, and it may differ between runs. Writers MUST include `engine` `{name, version}`, `algorithm` (§12), `adapter` `{id, version}` (`id` is `text`, `binary` or `tree`), `binding` (`rust`, `cli`, `python`, `node`, `wasm`, `java`, `c`, `server`), `platform` `{os, arch}` and `timing` `{totalMicros}`. Timing values appear only here. Writers MAY add more keys, such as library versions, the Unicode version or the detection ID (`detect.sniff@1`, §16.1). Readers MUST NOT use provenance to interpret the rest of the document.

## 12. Algorithm IDs

An algorithm ID has the form `<kind>.<name>@<n>`. The line diff IDs are `text.histogram@1`, `text.myers@1` and `text.patience@1`, chosen by `options.algorithm` and recorded as `provenance.algorithm`. Inline refinement (`text.refine@1`) and moved-block detection (`text.moves@1`, pure and edited moves) have their own IDs, defined in the moves spec. The binary adapter is `binary.bytes@1` (§17), recorded as `provenance.algorithm` for kind `binary`, the tree adapter `tree.entries@1` (§19), recorded for kind `tree`, and type detection is `detect.sniff@1` (§16.1). Writers record them in provenance when they run. Any change that alters the output for any input bumps `n`. That includes a library upgrade, a heuristic, a deterministic cost limit or a grouping rule. The bump is recorded in the changelog, and the corpus expected files are regenerated in the same change. The regex dialect and its Unicode tables come from the pinned `regex` crate, so an upgrade that changes any match, or the NFA unit count of any pattern (§7.2.1), bumps the line diff IDs too. A schema change on its own does not bump algorithm IDs.

## 13. Ordering and determinism

Arrays follow file position (hunks by `left.start`, ops in tiling order), with spans ordered as in §6.4, moves as in §6.5 and warnings as in §10. Hash-map order and locale never matter. The only non-integer numbers are `stats.similarity` and `moves[].similarity`, both basis points written as in §6.3. `bytes[]` follows file position, and `tree.entries` is sorted by path (§6.8). Outside `provenance`, the output depends only on the input bytes, the input names and the resolved options; for a folder, the inputs are the names, types, contents and link targets of the entries it lists (never timestamps, permissions, owners or the order the file system returns).

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
  "schemaVersion": "0.4.0",
  "id": "sha256:f221a785d0e51a6a0ab0887619d144522fd921910db3b2460b4865712f8de99e",
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
    "linesUnchanged": 4, "changeCount": 2, "hunkCount": 2, "similarity": 0.7273,
    "bytes": null, "tree": null
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
    "detectMoves": false,
    "adapter": "auto",
    "tree": {
      "followSymlinks": false, "ignoreFiles": false, "exclude": [],
      "detectMoves": true, "maxDepth": 3, "childBudget": 16777216
    }
  },
  "provenance": {
    "engine": { "name": "diffyman", "version": "0.1.0" },
    "algorithm": "text.histogram@1",
    "adapter": { "id": "text", "version": "0.1.0" },
    "binding": "rust",
    "platform": { "os": "macos", "arch": "aarch64" },
    "timing": { "totalMicros": 412 },
    "detect": "detect.sniff@1"
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

## 16. Type detection and adapter selection

### 16.1 Detection (`detect.sniff@1`)

Each input is detected on its own, from its bytes and the final component of its name. Nothing else (platform, locale, file system) is looked at. Two windows are used: the **sniff window** is the first 8000 bytes (the window of git's binary check) and the **magic window** the first 65536 bytes.

**Class.** An input is exactly one of:

1. **empty**: it has no bytes.
2. **text**, with encoding `utf-8`: after an optional EF BB BF, the input is valid UTF-8 (§7.1), and its sniff window holds no NUL byte and at most one **control byte** per 10 bytes (`10 × controls ≤ window length`). Control bytes are 0x01 to 0x08, 0x0E to 0x1A, 0x1C to 0x1F and 0x7F; TAB, LF, VT, FF, CR and ESC are not control bytes.
3. **text**, with encoding `utf-16le` or `utf-16be`: the input starts with FF FE or FE FF, the rest has even length and is valid UTF-16 (no unpaired surrogate), and the first 4000 code points hold no U+0000 and at most one control character (the same values as code points) per 10. A UTF-32 BOM (FF FE 00 00) therefore fails.
4. **binary**: anything else.

The class never depends on the name: renaming a file can change its media type, never how it is compared.

**Media type**, by class:

- **binary**: the first match of the pinned `infer` crate (0.22.0, built without its Compound File parser) over the magic window, in that crate's matcher order; else the binary entry of the extension table; else `application/octet-stream`.
- **text**: the text entry of the extension table; else `application/xml` when the decoded UTF-8 text starts with an XML declaration, or `text/html` when it starts with an HTML tag (the WHATWG MIME-sniffing patterns as implemented by `infer`, over the sniff window); else `text/plain`.
- **empty**: any entry of the extension table; else `text/plain`.

The **extension** is the part of the name after its last `.`, compared without ASCII case. A name without a dot, or whose only dot is its first character, has none. The extension table:

| Applies to | Extensions → media type |
|---|---|
| text and empty | `css` → `text/css`; `csv` → `text/csv`; `htm`, `html` → `text/html`; `js`, `mjs` → `text/javascript`; `json` → `application/json`; `log`, `txt` → `text/plain`; `markdown`, `md` → `text/markdown`; `svg` → `image/svg+xml`; `tsv` → `text/tab-separated-values`; `xml` → `application/xml`; `yaml`, `yml` → `application/yaml` |
| binary and empty | `bin` → `application/octet-stream`; `gif` → `image/gif`; `gz` → `application/gzip`; `jpeg`, `jpg` → `image/jpeg`; `pdf` → `application/pdf`; `png` → `image/png`; `wasm` → `application/wasm`; `webp` → `image/webp`; `zip` → `application/zip` |

Any other extension, such as `py` or `ini`, gives `text/plain` for text. A change to these rules, to the table or to the `infer` version bumps `detect.sniff@n`.

### 16.2 Adapter selection

`options.adapter` (§9) picks the kind:

| `adapter` | Kind |
|---|---|
| `auto` | `tree` when each input is an archive (§19.1); else `text` when each input is empty or text with encoding `utf-8`; otherwise `binary` |
| `text` | `text`, whatever the inputs; an input that is not UTF-8 gives `invalid-utf8` (§7.1). Valid UTF-8 with NUL bytes is compared as text. |
| `binary` | `binary`, whatever the inputs |
| `tree` | `tree`, whatever the inputs; an input that is not an archive gives `invalid-archive` (§10) |

Under `auto`, a binary document whose inputs include a text input gets the `binary-fallback` warning (§10). Byte-identical inputs give an `identical` document of the kind chosen here. Whether an input is an archive depends only on its first bytes (§19.1), never on its name. Two exceptions come from §19: inside a tree, `auto` gives `binary` instead of `tree` below `tree.maxDepth` (§19.5); and when the caller passes folders (an engine API that reads paths, such as the CLI), a pair with a folder is always kind `tree`, whatever `adapter` says, and its other side must be a folder or an archive.

## 17. Binary diff procedure (kind `binary`, `binary.bytes@1`)

The binary adapter compares the raw bytes of both inputs. Inputs over 4 GiB (2^32 bytes) per side give `limit-exceeded` (an error document with file meta and the echoed options).

1. Equal inputs give `bytes: []`. Otherwise the common prefix and then the common suffix of what is left are set aside.
2. When the rest of both sides together is at most 65536 bytes, it is aligned byte by byte: each byte value is a token, and the token sequences are diffed with the engine of `text.myers@1` (§12). Each maximal run of unmatched bytes between matched bytes (or the edges) becomes one range, as change ops are built in §7.3 step 2. One diff may align at most 2^32 **work units** byte by byte: a region of n left and m right bytes (after its own common prefix and suffix are set aside) costs n·m, and regions are taken in file order. A region with bytes on one side only is one range and costs nothing. A region over 65536 bytes, or one that does not fit what is left of the budget, goes on to step 3.
3. The region is cut into content-defined chunks with FastCDC 2020 (`fastcdc` 5.0.0) at the first **chunk level** it has not been cut at: level 1 has minimum 2048, average 8192 and maximum 65536 bytes; level 2 has 256, 1024 and 8192; level 3 has 64, 256 and 1024. A level is skipped when neither side is longer than its minimum (one chunk per side), and levels 2 and 3 are used only for regions of at most 64 MiB (2^26 bytes) per side. Chunks with equal bytes get the same token, in first-seen order (left, then right); BLAKE3 (`blake3` 1.8.7) finds candidates and the bytes are compared, so no hash collision can join different chunks. The token sequences are diffed with `text.histogram@1`. Each run of unmatched chunks is a changed region, handled from step 1's prefix and suffix rule on: aligned as in step 2 when it fits, and otherwise cut at the next level. So edits closer together than one chunk still get ranges of their own.
4. A region with bytes on both sides that step 2 does not align and that has no chunk level left is compared **position by position**: byte i of the left part against byte i of the right part, for i below the shorter length. Each run of at least 8 equal bytes stays equal, and everything between those runs is a range starting at the same offset on both sides; the bytes past the end of the shorter part join the last range. This result is used when the equal runs cover at least half of the shorter part, as for data overwritten in place; otherwise the region becomes one `replace` range. Either way `bytes-coarse-limit` is set.
5. Ranges separated by fewer than 8 equal bytes are joined into one range spanning both, so chance matches inside changed data do not split it.
6. While there are more than 100000 ranges, step 5 runs again with a gap of 16 bytes, then 32, doubling each time, and `bytes-coarse-limit` is set.

Every step is deterministic and depends only on the bytes. The work is bounded: byte alignment runs only on regions of at most 65536 bytes and within its work budget, each byte is chunked and hashed at most once per chunk level, each chunk alignment runs under the Histogram work budget, and the position-by-position comparison is linear. Stats follow §6.3.1.

## 18. Changes from 0.2.0

- New kind `binary` (§16, §17): byte ranges in `bytes[]` (§6.7, no longer reserved) and byte counts in the new field `stats.bytes` (§6.3.1, `null` for kind `text`).
- New option `adapter` (§9, §16.2), default `auto`, echoed in every document.
- `mediaType` is the detected media type (§16.1) instead of always `text/plain`. For text inputs this changes only the names in the extension table, for example `left.csv` is now `text/csv` and `left.yaml` `application/yaml`.
- New warning codes `binary-fallback` and `bytes-coarse-limit` (§10). `limit-exceeded` also covers the binary input size limit.
- Canonical numbers (§5): a value of magnitude 2^53 up to 10^21 that is not stored as an integer is rejected at write time, so canonical text is always a fixed point. No document this spec defines contains one.
- Type detection and the binary adapter have their own IDs, `detect.sniff@1` and `binary.bytes@1` (§12). The line diff IDs are unchanged: for two UTF-8 texts the hunks and stats are those of 0.2.0.
- `text.refine@1` and `text.moves@1` apply `ignore.regex` masks (§7.2.1). `text.moves@1` also aligns each `replace` op again by move key, so a whole-file terminator conversion no longer hides moves, compares lines longer than 256 graphemes by their common prefix and suffix, and requires a minimum word similarity for edited moves ([moves spec](text-granularity-and-moves.md) §4.1, §4.5). Neither ID had been released, so both stay at `@1`. Without masks, refinement output is unchanged.
- The message of `moves-skipped-limit` (§10) now covers detection that was cut short, not only skipped.

- New kind `tree` (§19): folders and archives (zip, tar, gzip- and zstd-compressed tar) compared entry by entry, with renames found by content, and a child document for each changed file. The `tree` block is no longer reserved (§6.8), `stats.tree` is new (§6.3.2, `null` for the other kinds), status `partial` is used for the first time (§6, §19.6), and file meta of a folder side is defined (§6.1).
- `adapter` gains the value `tree`, and under `auto` two archives give kind `tree` (§16.2). New options under `tree` (§9), echoed in every document.
- New warning codes `non-utf8-path`, `unsafe-path`, `duplicate-path`, `encrypted-entry`, `entry-unreadable`, `tree-limit`, `children-omitted`, `depth-limit`, `invalid-glob` and `invalid-archive` (§10). `limit-exceeded` also covers the tree limits (§19.6).
- The tree adapter has its own ID, `tree.entries@1` (§12).

The version bumps `y` (§2.1) although the shape only grows, as in 0.2.0: the same inputs and options can now give a different result. An input that is not valid UTF-8 gives a binary document instead of an `invalid-utf8` error document (pass `adapter: "text"` for the 0.2.0 result), and valid UTF-8 with a NUL byte in its first 8000 bytes, which 0.2.0 compared as text, is now binary. Two archives give a tree document instead of a binary one (pass `adapter: "binary"` for the byte ranges).

## 19. Folders and archives (kind `tree`, `tree.entries@1`)

The tree adapter compares two folders or archives entry by entry. It never extracts anything to disk: archive entries are read as in-memory streams, and entry paths are only compared as strings.

### 19.1 Sides

A side of a tree is one of:

- A **folder**: a directory given by path. Only engine APIs that read paths take one (the CLI, the Rust `diff_paths` function, the Python binding with folder paths); the WASM binding takes bytes only.
- An **archive**, recognized from its first bytes only:
  - zip: starts with `50 4B 03 04` (a local file header) or `50 4B 05 06` (the end record of an empty archive);
  - tar: at least 512 bytes, with `ustar` at bytes 257 to 261 (POSIX and GNU tar);
  - gzip-compressed tar: starts with `1F 8B`, and the first 512 bytes it decompresses to are a tar header as above;
  - zstd-compressed tar: starts with `28 B5 2F FD`, or with a zstd skippable frame (magic `5? 2A 4D 18`, a 4-byte little-endian length, then that many bytes), with the same rule. Skippable frames before, between and after the frames are stepped over, and one whose length runs past the end of the input makes the stream broken.

Other containers (bzip2, xz, 7z, a gzip stream that is not a tar) are not archives here and are compared as bytes.

**Reading paths.** The reference engine's path readers (the CLI, and the Rust `diff_paths` function, which the Python binding uses for folders) read each file that is not a folder whole when it is a regular file, and a pipe, a device or standard input, whose size is not known, up to 4 GiB (2^32 bytes, the largest binary or archive input, §17 and §19.6). A file paired with a folder is always read as an archive, so it is read only up to 4 GiB as well, even when it is a regular file. A caller can set a read limit for every such input (for a file paired with a folder, a limit over 4 GiB still reads at most 4 GiB); a regular file over it is refused by its size before it is read. An input over the limit is an error and no document is produced (its size and digest are not known, so there is no file meta to report): the reference engine's message starts with `limit-exceeded:`. Files inside folders are listed by streaming their content and are never read whole.

### 19.2 Listing a side

Each side becomes a list of entries `{path, type, size, sha256, target}` (§6.8).

**Folders.**

1. The folder is walked recursively. A symbolic link is listed as a `symlink`, with its link text as `target`, and never followed, unless `tree.followSymlinks` is `true`: then a link is listed as what it points to (a linked folder is walked), and a link that is broken or leads back into a folder being walked is listed as a `symlink`. The root folder itself is always followed.
2. Each name is one path component. A name that is not valid UTF-8 is decoded with every invalid sequence replaced by U+FFFD (`non-utf8-path`).
3. A file gets the `size` and `sha256` of its content. Pipes, sockets and devices are type `other` and are never opened. A file that cannot be read is listed with `sha256: null` and `size: null`, and a folder that cannot be read is listed but not walked (`entry-unreadable`). Folders have no content limit.
4. With `tree.ignoreFiles`, the `.gitignore` and `.ignore` files inside the folder exclude entries the way git applies `.gitignore`: one file per folder, relative to that folder, with globs, `!` negation and directory-only patterns, deeper files taking precedence, and `.ignore` over `.gitignore`. Ignore files above the root, the global git excludes file, `.git/info/exclude` and git configuration are never read, and no git repository is needed. The ignore files themselves are listed unless they exclude themselves.

**Archives.**

5. **zip** (`zip` 9.4.1): the entries of the central directory, in order. A name that is valid UTF-8 is read as UTF-8 and any other name as CP437, so zip names never need U+FFFD. A name ending in `/` is a `directory`; an entry whose Unix mode says symbolic link is a `symlink` whose target is its content (at most 4096 bytes, else `entry-unreadable`); every other entry is a `file`. Stored and deflate content is read and must match the declared size and CRC-32 (else `entry-unreadable`, like any other compression method). Encrypted entries are never decrypted (`encrypted-entry`).
6. **tar**: the entries in stream order, in POSIX ustar, GNU or pax form. Type flags `0`, NUL and `7` are files; `5` directories; `2` symlinks and `1` hardlinks, with the link name as `target`; any other flag is `other`, and its data is skipped. Pax headers (`x`, and `g`, which is read and ignored) and GNU long names (`L`, `K`) apply to the next entry and are not entries themselves; a pax `path`, `linkpath` or `size` record overrides the header field. The header checksum must match, sizes are octal or GNU base-256, an extension header holds at most 64 KiB, and the stream ends at a zero block or at the end of its data on a block boundary; anything else makes the side `invalid-archive`. Names are bytes, decoded as UTF-8 with U+FFFD for invalid sequences (`non-utf8-path`). The gzip layer may hold several members (`flate2` 1.1.11, pure Rust), the zstd layer several frames, each with a window of at most 64 MiB (`ruzstd` 0.9.1).
7. **Names** are normalized: `\` becomes `/`, the name is split at `/`, empty and `.` components are dropped, and `..` components are kept as they are, never resolved. A name that starts with `/` keeps one leading `/`. A name that normalizes to nothing (such as `./`) is not an entry. A path that starts with `/`, starts with a drive letter (`C:`), or has a `..` component gets `unsafe-path`; it is listed like any other path.

**Both.**

8. `tree.exclude` holds lines of a `.gitignore` file, matched against each path relative to its side's root: globs with `*`, `?`, `[...]` and `**`, `!` to re-include, a leading or inner `/` to anchor at the root, and a trailing `/` for directories only. An entry is left out when its path or a parent path matches. Excluded folders are not walked. A pattern that does not compile gives `invalid-glob`; more than 64 patterns, or a pattern longer than 1024 characters, gives `limit-exceeded`.
9. Every directory that holds a listed entry is listed too, as a `directory`, even when the archive has no entry for it.
10. When two entries of one side have the same path, the later one in stream or walk order is kept. A directory implied by rule 9 never replaces another entry. Either case, and a non-directory entry with entries below it, gives `duplicate-path` for that side.

Folders are walked in name order (names compared as bytes), so the result never depends on the order the file system returns, and nothing but names, types, contents and link targets is read: no timestamps, permissions or owners.

### 19.3 Errors

- A side that is neither a folder nor an archive (under `adapter: "tree"`, or a plain file paired with a folder), or an archive whose structure cannot be read (no central directory, a bad tar header, a broken compression layer), gives `invalid-archive` for that side. A broken entry inside a readable zip is `entry-unreadable`, not an error.
- More than 100000 entries on one side (implied directories included), more than 16 MiB of path text on one side, a zip whose end records declare more than 100000 entries, or an archive input over 4 GiB, gives `limit-exceeded`. The end records checked are the ones a zip reader can use: a zip32 end record (`50 4B 05 06`) that starts in the last 65621 bytes of the input, and a zip64 end record (`50 4B 06 06`) that a zip64 locator (`50 4B 06 07`) right before such a zip32 record names, starting at or after the locator's offset and ending, by its declared size, exactly at the locator. Signature bytes anywhere else, such as inside an entry's content, are data.

An error document keeps the file meta of both sides (§6.1); `tree` and `stats` are `null`.

### 19.4 Pairing

1. A path listed on both sides gives one entry. Different types give `typeChange`. Two files are `unchanged` when both have a `sha256` and they are equal; or when neither has one (content not read) and both are zip entries with the same compression method, CRC-32, sizes and stored bytes (an overlapping entry, §19.6, has no stored bytes to compare and is never `unchanged` this way). Otherwise they are `modified`. Two links are `unchanged` when their targets are equal, else `modified`. Two directories, or two `other` entries, are `unchanged`.
2. A path on the left only gives `removed`, on the right only `added`.
3. With `tree.detectMoves`, removed and added files that have a `sha256` and a `size` above 0 are paired when their `sha256` is equal. For each `sha256`, each removed file in path order takes the first added file, in path order, that has the same last path component and is not yet paired; then the remaining removed files take the remaining added files, both in path order. Each pair gives one `moved` entry at the right path, with the left path as `fromPath`. Directories, links and empty files are never moved.
4. Entries are sorted by `path` (§6.8).

### 19.5 Child documents

When `tree.childBudget` is above 0, every `modified` entry of two files that both have a `sha256` gets a child document, in path order, unless one of these leaves it out:

1. Either content is over 64 MiB, or the side is a compressed tar and the contents kept for this document's children would exceed 256 MiB on that side (contents are kept in path order of the entries), or reading the content of an archive side again would exceed what is left of that side's content budget (§19.6). Content read for a child counts against that budget, as the listing's does; content from a folder does not.
2. The child does not fit the budget: its size is the length of its canonical JSON without `provenance` (§5), and it is embedded only when that fits what is left of `tree.childBudget` for the top-level document. An embedded child uses up that much of the budget. Children of nested trees draw from the same budget, and a nested document's size includes its own children. A nested tree first sets aside its own **skeleton**: the length of its canonical JSON without `provenance`, with every `childDocument` null, a 71-character `id` and the `children-omitted` warning. Its children may use only what is left after that, so a nested tree whose skeleton fits always fits with the children it embeds; when its skeleton does not fit, it gets no children and is left out itself.

A child left out gives `children-omitted`. The child document compares the two contents with the parent's resolved options, except that `adapter: "tree"` becomes `auto`; its names are the last path components of the entry. The top-level document has depth 0 and a child has its parent's depth plus 1. In a child deeper than `tree.maxDepth`, two archives give kind `binary` with `depth-limit` instead of kind `tree`.

### 19.6 Limits and status

Every limit is deterministic and depends only on the inputs:

| Limit | Value | Effect |
|---|---|---|
| Entries per side, implied directories included | 100000 | `limit-exceeded` |
| Path text per side | 16 MiB | `limit-exceeded` |
| Archive input | 4 GiB | `limit-exceeded` |
| Extension header (pax, GNU long name) | 64 KiB | `invalid-archive` |
| One path | 4096 bytes | entry not listed, `tree-limit` |
| Content read from archives per side, all nesting levels of one top-level document, child contents included (§19.5) | 4 GiB | listing: `tree-limit`, zip file entries after it are listed without content, a tar stops there and lists no further entries; child contents: `children-omitted` |
| Compression ratio of a zip entry | declared size over 1 MiB and over 100 × its compressed size | not inflated, listed without content, `tree-limit` |
| Overlapping zip entries | an entry whose bytes (local header to the end of its stored data) start inside an earlier entry's, ordered by start offset, then end offset, then central directory order | neither inflated nor its stored bytes read, listed without content, `tree-limit` |
| Compression ratio of a compressed tar | over 1 MiB decompressed and over 100 × the compressed bytes read | the tar stops there, `tree-limit` |
| zstd window | 64 MiB | `invalid-archive` |
| Child input | 64 MiB per side; 256 MiB kept per compressed tar side | `children-omitted` |
| Nesting | `tree.maxDepth` | `depth-limit` |
| Child documents | `tree.childBudget` bytes | `children-omitted` |

The content of a file is **not read** when its entry has `sha256: null`: it is encrypted (`encrypted-entry`), could not be read (`entry-unreadable`), or a content limit stopped it (`tree-limit`). Such files are compared as in §19.4 rule 1, and get no child document. A tree whose warnings include `encrypted-entry`, `entry-unreadable` or `tree-limit`, and no error, has `status: "partial"`: the entries are valid, but equal content cannot be confirmed everywhere, or some entries are missing. `partial` takes precedence over `identical` and `different`, and the stats (§6.3.2) count the entries as listed. Child documents never change their parent's status.

`provenance.algorithm` is `tree.entries@1` and `provenance.adapter.id` is `tree`. A change to these rules, a limit, or the pinned `zip`, `flate2`, `ruzstd` or `ignore` versions bumps it.

### 19.7 Builds without archive support

An engine build MAY leave out archive support to be smaller (the WASM package built without its `archives` feature, [ADR 0008](../adr/0008-wasm-target.md)). Such a build never returns a different document: for every input pair and options it accepts, its document is conformant-equal (§3) to that of a full build. It refuses, before any comparison, exactly these requests with an `unsupported` error and no document, as the API rejects invalid options (§9):

1. `adapter` is `tree`;
2. `tree.exclude` is not empty (the patterns cannot be checked without the tree adapter);
3. `adapter` is `auto` and both inputs **start like an archive**: they begin with a zip local header or end-of-central-directory signature (`PK\x03\x04`, `PK\x05\x06`), hold `ustar` at byte offset 257 of a first 512-byte block, or begin with the magic number of a gzip stream (`1F 8B`), a zstd frame (`28 B5 2F FD`) or a zstd skippable frame (`5? 2A 4D 18`).

Every input §19.1 reads as an archive starts like one, so rule 3 covers every pair a full build compares as a tree without decompressing anything. It also refuses gzip or zstd data that holds no tar, which a full build compares as bytes. Folders need a file system and are never an input of such a build. The reference engine's error message starts with `unsupported:`, and bindings that report errors as values give the code `unsupported` (the WASM binding throws an `Error` with `name` `DiffymanError` and `code` `"unsupported"`). This is not a document warning code: no document carries it.

## 20. Changes from 0.3.0

- New field `moves[].inline` (§6.5): the inline spans of an edited move, in the `Span` shape of §6.4, computed by `text.refine@1` over the move's two ranges without the lines' indentation ([moves spec](text-granularity-and-moves.md) §4.7, [ADR 0011](../adr/0011-inline-spans-in-edited-moves.md)). It is `[]` for pure moves, under granularity `line`, over the refinement limits and under the `smart` noise cutoff. Ops still carry no spans for moved lines.
- `refine-skipped-limit` (§10) also covers edited moves over the refinement limits.
- `text.refine@1` and `text.moves@1` had not been released, so both stay at `@1`. Hunks, ops, stats and which moves are found are unchanged.
- Builds without archive support (§19.7) refuse the requests that need it with an `unsupported` API error instead of returning a document. No document field or code changes.

The version bumps `y` (§2.1) although the shape only grows, as in 0.2.0 and 0.3.0: the same inputs and options now give a different document whenever `detectMoves` finds a move, because every move gains `inline`.
