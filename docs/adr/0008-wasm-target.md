# 0008. WASM target: opt-level 3, name section removed, no wasm-opt

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

diffyman ships a browser and edge build (`diffyman-wasm`, the package wasm-pack builds from
`bindings/wasm`, used by `@diffyman/react`) from the same Rust core (ADR 0001, ADR 0007),
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
6. **Size budget.** The release gate stays: under 8 MiB raw. CI records raw, gzip and brotli
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
- Revisit when structural (tree-sitter) or format crates join the
  WASM build and push the size budget, when Safari numbers come in, or when wasm threads or memory64
  are needed for large files.

## Update (2026-11-07): `ignore.regex` shipped

Schema 0.2.0 supports `ignore.regex`, so regex stays in the WASM build with the same features as
native builds (decision 5): `unicode` for identical matches, `perf` because it changes speed only.
Pattern limits are counted in NFA units, which do not depend on pointer width, so 32-bit WASM accepts
and rejects exactly the patterns native builds do (corpus case `text/040-regex-limit-exceeded`).
The wasm-pack `--release` package (name section kept) now measures 1,791 / 577 / 412 KB raw, gzip and
brotli, up from 1,705 / 550 / 391 KB at Spike C, which also predates moved-block detection.

## Update (2026-11-08): archive support is a feature, on by default

Binary diff (ADR 0009) and folders and archives (ADR 0010) grew the module: zip, flate2, ruzstd,
`ignore` and the tar reader are all in the WASM build, because archives are read from bytes there too.
`bindings/wasm` now has a Cargo feature `archives` (engine feature `diffyman/archives`). Without it the
engine leaves out the tree adapter; binary diff stays, since any input that is not UTF-8 text needs it.

Sizes of the `--target web` module (wasm-pack `--release --no-opt`, then brotli 11 and gzip -9), as
built and with the name and producers sections removed as the release step does:

| Build | As built: raw / gzip / brotli | Release: raw / gzip / brotli |
|---|---|---|
| Before this change (archives always in) | 3,328 / 932 / 653 KB | 2,859 / 879 / 617 KB |
| `archives` on (default) | 3,330 / 932 / 653 KB | 2,860 / 880 / 618 KB |
| `--no-default-features` | 2,255 / 688 / 487 KB | 1,978 / 656 / 462 KB |

Leaving archives out saves 156 KB brotli (25%). The rest of the growth since Spike C (372 KB release)
is the binary adapter, type detection, moved blocks, slider compaction and regex, which every build
needs.

Decision:

1. **`archives` is on by default.** The package is a "diff any file" engine: `.docx`, `.xlsx`,
   `.jar`, `.epub` and `.apk` files are zip archives, and a browser tool is where people drop such
   files. With the feature on, the WASM binding gives the same document as the CLI and Python for every
   input that is not a folder, which keeps one conformance story. 618 KB brotli loads once, in a Worker
   (decision 7), and is far inside the 8 MiB raw budget (decision 6).
2. **Embedders that only diff text and bytes build `--no-default-features`** (462 KB brotli). That
   build never returns a different document; it refuses what needs archives with a typed error
   ([spec §19.7](../spec/diff-document.md)): adapter `tree`, a non-empty `tree.exclude`, or two inputs
   that both start like an archive (zip, tar, gzip or zstd signatures, so also plain gzip data, which
   it cannot tell from a compressed tar without the decompressors). The binding throws an `Error`
   with `name` `DiffymanError` and `code` `"unsupported"` (invalid options get `"invalid-options"`),
   and `features()` lists `["archives"]` or `[]`.
3. **CI builds both.** `run_bindings.py` asks the package for its features. Against the default
   package every case without a folder must match; against `pkg-node-slim` the 18 cases §19.7 covers
   (17 tree cases and `binary/016-gzip-vs-zip`) must give the `unsupported` error and all others must
   still match byte for byte.

Rejected:

- **Off by default.** Smaller, but two archives would be an error in the package most people install,
  and the default build would no longer agree with the CLI on 18 corpus cases.
- **Detecting compressed tars exactly without archives.** It needs the gzip and zstd decoders, which
  are a large part of what the feature saves.
- **Two npm packages.** Doubles the release work for one feature; a build flag serves the embedders
  that need the smaller module.
