# diffyman-render

Renderers for diffyman Diff Documents ([spec](../../docs/spec/diff-document.md)). Each renderer works from a finished document and never re-runs the diff. Renderers that show content also take the two inputs, because the document holds line positions, not text. The size and SHA-256 of each input are checked against the document's file meta.

A caller that has just computed the document from the same bytes in the same process (the CLI) uses `unified_computed` and `html_computed` instead, with the inputs as `Computed`: the bytes plus the decoded inputs the engine returned (`diffyman::diff_document_with_text`, each text with its line index). The output is the same, without a second SHA-256 of both inputs, UTF-8 check and line table: on the 1 GB log pair the text output takes 2.1 s and 0.59 GB instead of 3.6 s and 1.0 GB, about what `--json` costs. Sizes, line counts, that each text fills its input, and the hunk structure are still checked, so a document that does not fit is `RenderError::InvalidDocument` and never a panic. Documents from anywhere else go through `unified` and `html`, which keep the SHA-256 check. Lines are split by `diffyman_model::lines`, the same code the engine uses.

| Function | Output |
|---|---|
| `unified(doc, left, right, &UnifiedOptions)` | Unified diff that `patch` and `git apply` accept |
| `html(doc, left, right, &HtmlOptions)` | One self-contained HTML report (text, binary and tree documents) |
| `html_with_contents(doc, left, right, &mut contents, &HtmlOptions)` | The same, with the content of tree entries from an `EntryContents` (any `FnMut(&[&str]) -> Option<(Vec<u8>, Vec<u8>)>`) |
| `summary(doc, &SummaryOptions)`, `summary_line(doc)` | A verdict line plus stats (text, binary and tree documents) |
| `quote_path(name)` | A name quoted like git when it holds control or bidi characters |

Output depends only on the document (without `provenance`), the input bytes and the options. The crate reads no clock, locale or environment, and it builds for `wasm32-unknown-unknown`.

## Options

- **Context.** `context: None` keeps the document's hunks. Another value regroups the document's ops with the rules of spec §7.4. With fewer lines than the document's `options.context` the result is the same as running the engine with that context. With more lines, the lines between hunks are shown as context, except when the document used `ignore.blankLines`: those lines may hold `ignored` ops that the document does not list, so context then stops at the document's hunks.
- **Color.** `UnifiedOptions::color` is a plain flag. `ColorChoice::resolve(is_terminal, no_color)` follows the [`NO_COLOR`](https://no-color.org) convention: `Auto` colors only on a terminal and only when `NO_COLOR` is unset or empty, and `Always` overrides `NO_COLOR`. The colors are git's defaults (bold headers, cyan hunk headers, red `-`, green `+`).
- **Terminal safety.** `UnifiedOptions::escape_controls` shows control, bidi-control and invisible characters in content as `<ESC>`, `<RLO>`, `<U+0085>` and so on. Use it for display only: the output is then no longer a byte-exact patch. Names in headers and summaries are always quoted like git when they hold such characters.

## Unified diff

- Headers are `--- a/<left name>` and `+++ b/<right name>` (or the given labels). There is no `diff --git` line, no `index` line and no timestamp.
- Hunk headers follow GNU: `@@ -3 +3 @@` for one line, `@@ -0,0 +1,2 @@` for an insertion into an empty side.
- Line bytes are kept: CRLF stays CRLF, a missing final newline gives `\ No newline at end of file`, and a BOM is written at the start of line 1 of its side.
- `replace` and `ignored` ops are written as `-` lines, then `+` lines.
- An identical document gives an empty string. A document with an `invalid-utf8` error gives `Binary files a/... and b/... differ` when the bytes differ. Other error documents are `RenderError::ErrorDocument`.

### Compatibility with git and diff

`tools/compare_unified.py` renders every text corpus case and compares the result with `git diff --no-index --no-color` (git's default slider compaction and indent heuristic, which the engine follows, spec §7.3.1; same algorithm, context and whitespace flags; `diff --git`, `index` and mode lines dropped, and the function context after the second `@@` removed) and with `diff -u -L a/<left> -L b/<right>` (BSD diff on macOS). It also applies each patch to the left file with `patch` and with `git apply` and checks that the result equals the right file byte for byte.

```
uv run --python 3.12 crates/diffyman-render/tools/compare_unified.py
```

Last run (git 2.54, BSD diff, 42 text cases): 25 of 36 comparable cases match git exactly and 26 of 33 match `diff -u`. Cases with `ignore.case`, `ignore.regex` or an error document are skipped. Every patch made without ignore options reproduces the right file with both `patch` and `git apply`, apart from the cases listed below. The other differences, all expected:

| Difference | Cases | Why |
|---|---|---|
| Context lines under an ignore option come from the left file; git takes them from the right file | 007, 011, 016, 029, 030 | A patch is applied to the left file, so its context must match the left file. BSD and GNU `diff` do the same. |
| A lone CR ends a line in diffyman (spec §7.2) but not in git | 023, 024, 025 | diffyman writes such a line with its CR followed by LF, as BSD `diff` does. Applying the patch turns those lone CRs into CRLF. |
| A difference only in the UTF-8 BOM is not a change | 010 | The document is `identical` (spec §6), so the patch is empty. |
| Inputs that are not UTF-8 give `Binary files ... differ` | 021, 022 | Under the default `adapter: "auto"` such a pair is a binary document (spec §16.2); these two cases pin `adapter: "text"` and are `invalid-utf8` error documents, rendered the same way. git does the same for 022 (it has NUL bytes) but diffs 021 as text. A binary document (kind `binary`) gives the same line when its bytes differ; its HTML report has a hex view. |
| Different but equally valid edit scripts | 007, 037 | `-b` treats VT and FF as whitespace in diffyman but not in git (007), and `ignore.blankLines` follows spec §7.3 rather than git's `--ignore-blank-lines` grouping (037). Slider positions match git's since the slider compaction of spec §7.3.1; BSD `diff` places some sliders differently (017, 042). |
| A zero-context patch needs `git apply --unidiff-zero` | 019 | As for git's own zero-context patches. |

## HTML report

- One file with inline CSS. There is no script and no external resource. The side-by-side and inline views are switched with radio buttons and CSS. `HtmlView` can keep only one of them.
- The markup is semantic: one table per view with column headers, one `<tbody>` per hunk with a row-group header, and `<del>`/`<ins>` for inline changes.
- Changes are marked with `−`, `+`, `⇄` (moved) and `≈` (ignored), plus hidden text for screen readers, so color is never the only signal. The palette uses orange for removed, blue for added and purple for moved, which stays distinguishable with the common forms of color blindness. A dark palette applies under `prefers-color-scheme: dark`.
- Moved blocks get a link at each end that points to the other end, and they are listed in the header. A moved line shows the inline spans of its move (`moves[].inline`, spec §6.5), every other changed line those of its op.
- All content and names are escaped. Control, zero-width and bidi-control characters are replaced by visible labels (`RLO`, `ZWSP`, `U+0085`), so they cannot reorder the page. A notice says when the content holds any. Content cells use `unicode-bidi: plaintext`, so right-to-left lines display in their own direction.
- When the two files use different line terminators, changed lines show `␊`, `␍␊` or `␍`. Tabs and trailing whitespace on changed lines are marked.
- Size limits: `max_bytes` (default 8 MiB) stops adding rows and says how many hunks are shown, and `max_line_bytes` (default 10000) cuts long lines at a character boundary.

### Binary documents

- A summary (ranges, bytes added, removed and unchanged, similarity), the media type, size, detected encoding and SHA-256 of both inputs, and the warnings.
- A table with one row per byte range: its number, the change (`+` inserted, `−` deleted, `±` replaced), and for each side the length and offset (hex and decimal) with a hex dump: whole rows of 16 bytes aligned to multiples of 16, the range's bytes in `<del>` or `<ins>` in both the hex and the text column, and `.` for bytes outside printable ASCII. An insertion point shows its offset without a dump.
- Limits: each side of one range shows at most 16 rows (256 bytes) with a note of how many bytes are left out, and the whole report at most `max_hex_bytes` (default 64 KiB) of dump; later ranges are listed without one and a notice says so. Rows stop at `max_bytes`. Ranges outside the inputs or with the wrong shape are `RenderError::InvalidDocument`.

### Tree documents

- A summary (entry counts by status, similarity), the warnings that matter for folders and archives flagged at the top with an explanation (`unsafe-path`, `encrypted-entry`, `entry-unreadable`, `tree-limit`, `duplicate-path`, `non-utf8-path`, `children-omitted`, `depth-limit`), both sides' name, type, entry count, size and digest, and the full warnings list.
- A table of the changed entries in path order: status icon and label (`+` added, `−` removed, `~` modified, `→` moved, `≠` type changed), the path (`old → new` for a move), each side's type and size or link target, and a short SHA-256 with the full digest as its tooltip. An unsafe path (absolute, a drive letter, a `..` component) gets an `unsafe path` badge, and a file whose content was not read a `content not read` badge. The unchanged entries follow in a collapsed list.
- Under each modified entry, its child document in a collapsed `<details>`: a text child as an inline view, a binary child as its byte-range table, a nested archive as its own entries table, each with its warnings. Children are shown down to `max_depth` levels (default 4), and every part of the page draws from the one `max_bytes` budget.
- The inputs are not read; entry contents come from `html_with_contents`. They are asked for by path (the entry's path in each enclosing tree, outermost first), only for entries the report shows, within `max_content_bytes` (default 256 MiB), and checked against the child document's size and SHA-256. Without content, a text child lists its hunks by line number and a binary child its ranges without a dump. The CLI supplies the contents of two folders' top-level entries; archive entries have none, because the engine does not keep them after the comparison (see [docs/cli.md](../../docs/cli.md#output-formats)).

## Tests

- `tests/render.rs` covers byte-exact patches, regrouping against the engine on every corpus case, escaping of hostile names and content (allowlisted tags and attributes only, no script, no dangling links), size caps, and documents that do not fit their inputs. For every corpus case on files, the `*_computed` output equals the checked output, with and without the engine's decoded inputs. Every corpus case (text, moved, binary and tree) renders as HTML that passes the same checks; hostile tree paths, link targets and digests are escaped; hex dumps, entry contents, nesting depth and tree size are capped.
- `tools/check_html.py` parses every stored report and a fresh CLI report for every corpus case with Python's `html.parser`: elements closed in order, allowlisted tags and attributes, no script, event handler, `src` or style attribute, one `<style>` that loads nothing, unique ids and in-page links only, and no bidi-control, control or invisible character in text or attributes:

  ```
  uv run --python 3.12 crates/diffyman-render/tools/check_html.py
  ```
- `tests/snapshots/` holds the exact output for a few corpus cases. After an intended change, rewrite the snapshots and review the diff:

  ```
  cargo test -p diffyman-render --test snapshots -- --ignored bless
  ```
