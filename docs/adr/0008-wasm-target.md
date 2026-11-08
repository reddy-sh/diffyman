# 0008. WASM target: opt-level 3, name section removed, no wasm-opt

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

The plan ships a browser and edge build (`@diffyman/wasm`) from the same Rust core (ADR 0001, ADR 0007),
and later a `wasm32-wasip1` build for Go and Java without native code. Spike B proved the WASM binding
returns the same document as the CLI. Spike C ([results](../spikes/C-wasm-build.md)) measured size,
speed and browsers, and checked the planned crates on `wasm32-unknown-unknown`:

- Today's package is 1,705 KB raw, 550 KB gzip, 391 KB brotli. 199 KB of it is the `name` section.
- Removing the `name` and `producers` sections gives 1,506 / 525 / 372 KB with identical glue.
  `wasm-opt` (binaryen 132) shrinks the raw file more but not the compressed one, and not the run time.
- opt-level `"s"` and `"z"` are 12% and 15% smaller compressed, but 17 to 24% and 50% slower.
- `strip = true` in Cargo also drops `target_features`, so wasm-bindgen 0.2.130 falls back to another
  ABI and warns.
- WASM is 1.5 to 1.7 times slower than the native engine: a 10 MB pair takes 114 ms in Node, 121 ms
  in Chrome 157 and 134 ms in Firefox 157.
- regex and its Unicode tables are about half the module, while `ignore.regex` is not supported yet.
- All 13 planned crates compile and link for `wasm32-unknown-unknown`. tree-sitter's C code needs
  `llvm-ar` and wasm libc headers for grammars; zip's default features add JS-only imports and C.

## Decision

1. **Target and binding.** The browser, Node and edge package is `wasm32-unknown-unknown` with
   wasm-bindgen (pinned, now 0.2.130), built for `--target web` and `--target nodejs`. No threads,
   no SIMD and no memory64 for now. The feature baseline is rustc's default wasm32 set (bulk memory,
   multi-value, reference types, sign extension, non-trapping float-to-int, mutable globals).
2. **Profile.** The WASM build uses the workspace `release` profile unchanged: opt-level 3, thin LTO,
   `codegen-units = 1`. Speed matters more than 46 to 54 KB of brotli.
3. **Post-processing.** Release packages run `wasm-bindgen --remove-name-section
   --remove-producers-section` with `wasm-bindgen-cli` pinned to the crate's wasm-bindgen version.
   We do not run `wasm-opt` and do not set `strip = true`. wasm-pack stays for local development with
   `--release` (its custom-profile path runs `wasm-opt` regardless of our metadata).
4. **Dependencies.** Anything that goes into the WASM build must pass `cargo build --target
   wasm32-unknown-unknown` in CI. Prefer pure Rust; use `default-features = false` with explicit
   features when defaults pull in `getrandom`, `time`, `js-sys` or C (zip). C dependencies (tree-sitter
   grammars) build with clang and `llvm-ar`, and vendored grammars add tree-sitter's wasm headers.
5. **regex keeps `unicode` everywhere**, so a pattern means the same thing on every surface. Whether
   the WASM build drops regex's `perf` features, or leaves regex out until `ignore.regex` ships, is
   decided with that feature, using the spike's numbers.
6. **Size budget.** The plan's release gate stays (under 8 MiB raw). CI records raw, gzip and brotli
   size of the release package; the baseline is 1,506 / 525 / 372 KB.
7. **Large inputs.** Browsers run the engine in a Worker with async `init()`. GB-scale work stays in
   the CLI and the local API service.

Rejected:

- **opt-level `"s"`/`"z"`.** Smaller, but up to 50% slower for every user to save 54 KB once.
- **`wasm-opt`.** No transfer-size gain, an extra downloaded binary, and feature flags to keep in sync.
- **`strip = true`.** Same saving, but changes wasm-bindgen's ABI and output.
- **Dropping regex `unicode`.** Smaller, but `\d`, `\w` and case folding would differ from native.

## Consequences

- The release job owns one more step (wasm-bindgen CLI) and one more pinned tool; its version must
  move with the `wasm-bindgen` crate.
- Packages carry no function names, so browser stack traces show indices. Debug builds keep names.
- CI needs clang and `llvm-ar` on Linux once tree-sitter joins the WASM build.
- Revisit when regex ships for `ignore.regex`, when structural (tree-sitter) or format crates join the
  WASM build and push the size budget, when Safari numbers come in, or when wasm threads or memory64
  are needed for large files.
