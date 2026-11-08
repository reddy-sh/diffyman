# Status

_Updated 2026-11-08._

| Gate | State | Notes |
|---|---|---|
| **Phase 0 — Foundations** | ✅ Done | CI green on Linux, macOS and Windows |
| **Phase 1 — Engine + CLI** | 🟡 In progress | Increment 1a done (line-diff core, moves, ignore rules, schema 0.2.0); 1b next |
| Gate M — headless MVP | ⚪ Not started | |
| Gate H — headless solid | ⚪ Not started | |
| Gate P — feature parity | ⚪ Not started | |

## Done in Phase 0

- **Spec:** [Diff Document v0](spec/diff-document.md) (`schemaVersion` 0.1.0) and [text granularity and moved blocks](spec/text-granularity-and-moves.md).
- **Decisions:** [ADR 0001–0008](adr/README.md) — Rust core, the Diff Document contract, Postgres, object storage, license, line-diff library, binding strategy, WASM target.
- **Workspace:** `diffyman-model`, `diffyman-algo`, `diffyman-engine`, `diffyman-cli`, `diffyman-conformance`; fmt, clippy (pedantic, `-D warnings`), 90+ tests and `cargo deny` pass.
- **Corpus:** 20 text cases and 40 hand-labelled moved-block cases; 60/60 pass in strict conformance.
- **Spike A — line diff** ([report](spikes/A-line-diff.md)): a 1 GB log pair diffs in about 1.5 s with about 400 MB footprint (budget: 60 s, 2 GB).
- **Spike B — bindings** ([report](spikes/B-binding-round-trip.md)): CLI, Python (PyO3, abi3) and WASM return byte-identical documents for every case.
- **Spike C — WASM** ([report](spikes/C-wasm-build.md)): 372 KB brotli; 1.5–1.7× native speed in Node, Chrome and Firefox; every planned format crate builds for `wasm32-unknown-unknown`.
- **Local stack:** `scripts/local-up.sh` starts PostgreSQL 18.6 on `127.0.0.1:5433`.

## Done in Phase 1

- **Ignore rules, schema 0.2.0** ([spec §7.2.1, §7.3](spec/diff-document.md)): `ignore.regex` masks each match with a per-pattern placeholder (a masked value still differs from a missing one), with deterministic limits (16 patterns, 1024 characters, 50000 NFA units) and a new `invalid-regex` error. `ignore.blankLines` turns blank-only changes into the new `ignored` op kind: not a change, not counted in stats, never a hunk of its own, and left out of the similarity denominator, so blank-only differences are `identical`. Re-blessing changed only `schemaVersion` and `id` in 74 of the 75 existing golden files; the other one, `text/018`, now masks its timestamps instead of being an `unsupported-option` error. Five new text cases (036–040); CLI, Python and WASM stay byte-identical on all 80 cases.
- **Canonical JSON writer:** documents are serialized straight to RFC 8785 text without a `serde_json::Value` tree. Two 32 MB inputs with a 107 MB document now peak at 0.6 GB footprint instead of 2.5 GB, with byte-identical output.
- **Moved-block detection** (`text.moves@1`, [moves spec §4](spec/text-granularity-and-moves.md)): pure moves (with indentation changes) and edited moves, `detectMoves: true` is supported, and moves never change hunks. On the 41 labelled cases: precision 1.00 and recall 1.00 (36 of 36 labels), kind accuracy 1.00, exact boundaries 0.81, no moves in the 9 negative cases. The conformance runner now gates on these metrics. Pure pieces of one moved block with an edited line inside are joined into one edited move (moves spec §4.3 step 4, case `moved/041`). Work is bounded by deterministic limits (`moves-skipped-limit`); the edited search compares small segment pairs first, so large unrelated churn no longer switches it off.

- **Line-diff core** ([ADR 0006](adr/0006-line-diff-library.md)): `text.histogram@1` and `text.myers@1` run on `gix-imara-diff` 0.3.3; Histogram has a deterministic work budget with a Myers fallback (200k swapped line pairs: 0.46 s instead of 33 s). Inputs over 256 MiB per side give `limit-exceeded`.
- **Spec alignment:** one source of truth for output shapes, options and warning codes ([diff-document.md](spec/diff-document.md)); LF, CRLF and lone CR all end a line; default granularity `line`.
- **Corpus:** 40 text cases (invalid UTF-8, UTF-16, mixed and lone-CR line endings, long lines, word/char/smart granularity) and 41 moved-block cases; 81/81 pass in strict conformance, which CI now enforces.
- **Canonical writer:** a direct RFC 8785 writer replaced the intermediate JSON tree; peak memory on a 1M-line, 107 MB result fell from 2.5 GB to 0.6 GB.

## Next (Phase 1b)

1. Large-file mode: stream or mmap inputs and cut per-line overhead (the 1 GB log pair takes 10.4 s and 5.7 GB through the CLI today; the line diff itself is 0.33 s).
2. Folders and archives (`tree`), hex + metadata fallback for binary files, type detection.
3. CLI: unified and HTML output, `dir`, `schema`, `formats`, `textconv`, git difftool integration.
4. Fuzzing (`cargo-fuzz`) and benchmarks in CI.
5. Move detection edge cases: masked-but-different lines under `ignore.regex`, very long lines with small edits, whole-file CRLF conversion under whitespace `none`, config sections whose values all changed; more moved-block cases from real repositories; a slider heuristic so closing braces stop landing in `equal` ops.
6. Inline refinement does not apply `ignore.regex` masks yet.

## Open items

- Safari has not been measured for the WASM build.
- License files still need to be added to the WASM and Python packages before any registry release.
