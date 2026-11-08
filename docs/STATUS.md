# Status

_Updated 2026-11-07._

| Gate | State | Notes |
|---|---|---|
| **Phase 0 — Foundations** | 🟡 Nearly done | Everything below is green locally; waiting for the first CI run on GitHub |
| Gate M — headless MVP | ⚪ Not started | Phase 1 (engine + CLI) is next |
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

## Next (Phase 1)

1. Switch the line diff to `gix-imara-diff` with a Histogram work budget (ADR 0006); `imara-diff` 0.2.0 is quadratic on very repetitive input.
2. Moved-block detection (currently a stub; recall 0 on the labelled cases).
3. Align the two spec documents (lone CR handling, default granularity, move fields) and add corpus cases for invalid UTF-8, mixed line endings and lone CR.
4. `ignore.blankLines` and `ignore.regex`; word/char/smart granularity out of "provisional".
5. Large-file mode, folders, hex fallback, fuzzing and benchmarks in CI.

## Open items

- Safari has not been measured for the WASM build.
- License files still need to be added to the WASM and Python packages before any registry release.
