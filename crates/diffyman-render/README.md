# diffyman-render

Renderers for diffyman Diff Documents ([spec](../../docs/spec/diff-document.md)). Each renderer works from a finished document and never re-runs the diff. Renderers that show content also take the two inputs, because the document holds line positions, not text. The size and SHA-256 of each input are checked against the document's file meta.

| Function | Output |
|---|---|
| `unified(doc, left, right, &UnifiedOptions)` | Unified diff that `patch` and `git apply` accept |
| `html(doc, left, right, &HtmlOptions)` | One self-contained HTML report |
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

`tools/compare_unified.py` renders every text corpus case and compares the result with `git diff --no-index --no-color --no-indent-heuristic` (same algorithm, context and whitespace flags; `diff --git`, `index` and mode lines dropped, and the function context after the second `@@` removed) and with `diff -u -L a/<left> -L b/<right>` (BSD diff on macOS). It also applies each patch to the left file with `patch` and with `git apply` and checks that the result equals the right file byte for byte.

```
uv run --python 3.12 crates/diffyman-render/tools/compare_unified.py
```

Last run (git 2.54, BSD diff, 40 text cases): 24 of 35 comparable cases match git exactly and 26 of 32 match `diff -u`. Cases with `ignore.case`, `ignore.regex` or an error document are skipped. Every patch made without ignore options reproduces the right file with both `patch` and `git apply`, apart from the cases listed below. The other differences, all expected:

| Difference | Cases | Why |
|---|---|---|
| Context lines under an ignore option come from the left file; git takes them from the right file | 007, 011, 016, 029, 030 | A patch is applied to the left file, so its context must match the left file. BSD and GNU `diff` do the same. |
| A lone CR ends a line in diffyman (spec §7.2) but not in git | 023, 024, 025 | diffyman writes such a line with its CR followed by LF, as BSD `diff` does. Applying the patch turns those lone CRs into CRLF. |
| A difference only in the UTF-8 BOM is not a change | 010 | The document is `identical` (spec §6), so the patch is empty. |
| Inputs that are not UTF-8 give `Binary files ... differ` | 021, 022 | Under the default `adapter: "auto"` such a pair is a binary document (spec §16.2); these two cases pin `adapter: "text"` and are `invalid-utf8` error documents, rendered the same way. git does the same for 022 (it has NUL bytes) but diffs 021 as text. A binary document (kind `binary`) gives the same line when its bytes differ; there is no hex view yet, so HTML returns `UnsupportedKind` for it. |
| Different but equally valid edit scripts | 007, 017, 037 | The line diff is the engine's; git and BSD diff place some changes differently (slider position, `-b` treats VT and FF as whitespace in diffyman but not in git, and `ignore.blankLines` follows spec §7.3 rather than git's `--ignore-blank-lines` grouping). |
| A zero-context patch needs `git apply --unidiff-zero` | 019 | As for git's own zero-context patches. |

## HTML report

- One file with inline CSS. There is no script and no external resource. The side-by-side and inline views are switched with radio buttons and CSS. `HtmlView` can keep only one of them.
- The markup is semantic: one table per view with column headers, one `<tbody>` per hunk with a row-group header, and `<del>`/`<ins>` for inline changes.
- Changes are marked with `−`, `+`, `⇄` (moved) and `≈` (ignored), plus hidden text for screen readers, so color is never the only signal. The palette uses orange for removed, blue for added and purple for moved, which stays distinguishable with the common forms of color blindness. A dark palette applies under `prefers-color-scheme: dark`.
- Moved blocks get a link at each end that points to the other end, and they are listed in the header.
- All content and names are escaped. Control, zero-width and bidi-control characters are replaced by visible labels (`RLO`, `ZWSP`, `U+0085`), so they cannot reorder the page. A notice says when the content holds any. Content cells use `unicode-bidi: plaintext`, so right-to-left lines display in their own direction.
- When the two files use different line terminators, changed lines show `␊`, `␍␊` or `␍`. Tabs and trailing whitespace on changed lines are marked.
- Size limits: `max_bytes` (default 8 MiB) stops adding rows and says how many hunks are shown, and `max_line_bytes` (default 10000) cuts long lines at a character boundary.

## Tests

- `tests/render.rs` covers byte-exact patches, regrouping against the engine on every corpus case, escaping of hostile names and content (allowlisted tags and attributes only, no script, no dangling links), size caps, and documents that do not fit their inputs.
- `tests/snapshots/` holds the exact output for a few corpus cases. After an intended change, rewrite the snapshots and review the diff:

  ```
  cargo test -p diffyman-render --test snapshots -- --ignored bless
  ```
