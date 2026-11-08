# Spike C: WASM build (size, speed, browsers, crates)

- **Date:** 2026-11-07
- **Question:** How big and how fast is the `wasm32-unknown-unknown` build of the engine, does it run in
  browsers, and which of the planned crates compile to `wasm32` today? The decision it feeds is
  [ADR 0008](../adr/0008-wasm-target.md).
- **Code:** [`bindings/wasm`](../../bindings/wasm), bench scripts in
  [`bindings/wasm/bench`](../../bindings/wasm/bench)
- **Decision:** [ADR 0008](../adr/0008-wasm-target.md)

## Summary

1. **Size.** Today's package (`wasm-pack --release`, no `wasm-opt`) is **1,705 KB raw, 550 KB gzip -9,
   391 KB brotli 11**. Dropping the `name` and `producers` custom sections after wasm-bindgen gives
   **1,506 / 525 / 372 KB** with identical code. That is the recommended build.
2. **wasm-opt does not pay off.** binaryen 132 at `-O3`, `-Os` or `-Oz` shrinks the raw file further
   (1,414 to 1,426 KB) but the compressed size is the same or slightly larger than just removing the name
   section (524 to 526 KB gzip, 376 KB brotli). Speed is unchanged.
3. **opt-level "s"/"z" are smaller but slower.** The smallest build is opt-level `"z"` with the name
   section removed: **1,321 / 438 / 318 KB**, 15% smaller brotli than the recommended build. It is 50%
   slower on both inputs. opt-level `"s"` is 12% smaller (326 KB brotli) and 17 to 24% slower. Fat LTO
   makes no difference to thin LTO (the profile already has `codegen-units = 1`).
4. **regex is about half the module.** regex code (regex-automata, regex-syntax, aho-corasick) is 518 KB
   of the 1,705 KB, and its Unicode tables are most of the 502 KB `.rodata` segment. Turning off regex's
   `perf` features (keeping `unicode`, so patterns mean the same thing) gives **1,175 / 412 / 293 KB**,
   but regex matching then runs 14 to 24 times slower. `ignore.regex` returns `unsupported-option` in
   schema 0.1.0, so no diff runs regex yet.
5. **Speed.** In Node 22, WASM takes **10.1 ms** for a 1 MB pair and **114 ms** for a 10 MB pair,
   against **6.0 ms** and **76 ms** for the native engine in process (1.5 to 1.7 times slower). Chrome
   157 (11.4 / 121 ms) and Firefox 157 (12 / 134 ms) are close to Node. Every surface returns the same
   document.
6. **Browsers.** Chrome and Firefox both ran headless with one command each. Safari was **not
   measured**: it has no headless mode, and `safaridriver` needs a one-time manual setting.
7. **Crates.** All 13 planned crates pass `cargo check` and `cargo build` (linked `cdylib`) for
   `wasm32-unknown-unknown`. Two need care: **tree-sitter** (C code) runs only with `llvm-ar` and
   wasm libc headers for grammars, and **zip** with default features pulls JS-only imports
   (`getrandom`, `time`) and C (`zstd-sys`).

## Setup

Development Mac (Apple M4 Max, macOS 27.0), rustc 1.99.1, wasm-bindgen 0.2.130, wasm-pack 0.15.1,
binaryen `wasm-opt` 132 (npm `binaryen@132.0.0`, local only), Node v22.23.4, Chrome 157 (headless),
Firefox 157.0.1 (headless), CPython 3.12 with `brotli==1.1.0`. Sizes are in KB = 1,000 bytes.

Inputs come from [`gen_inputs.py`](../../bindings/wasm/bench/gen_inputs.py): source-like lines of 40 to
100 characters with some non-ASCII words, and about 1% of lines edited, inserted or deleted on the right.

| Pair | Left | Right | Lines (left) | Hunks | Document |
|---|---|---|---|---|---|
| 1 MB | 1,000,029 B | 1,000,900 B | 12,784 | 115 | 49 KB |
| 10 MB | 10,000,042 B | 10,010,367 B | 127,671 | 1,189 | 499 KB |

All runs use the default options (`histogram`, line granularity).

## 1. Size

[`size.py`](../../bindings/wasm/bench/size.py) builds the `--target web` package for each profile
variant, then derives one file without the `name` and `producers` sections and three `wasm-opt` files.
It loads **every** file in Node ([`check_web_pkg.mjs`](../../bindings/wasm/bench/check_web_pkg.mjs))
and requires the same document for a fixed input pair. All 45 files produced the same document.

| Profile (all `codegen-units = 1`) | Post-processing | Raw | gzip -9 | brotli 11 |
|---|---|---|---|---|
| opt-level 3, thin LTO (today's `release`) | none (today's package) | 1,705 KB | 550 KB | 391 KB |
| opt-level 3, thin LTO | **name + producers sections removed** | **1,506 KB** | **525 KB** | **372 KB** |
| opt-level 3, thin LTO | `wasm-opt -O3` | 1,426 KB | 526 KB | 376 KB |
| opt-level 3, thin LTO | `wasm-opt -Oz` | 1,414 KB | 524 KB | 376 KB |
| opt-level 3, fat LTO | name section removed | 1,506 KB | 525 KB | 372 KB |
| opt-level `"s"`, thin LTO | none | 1,787 KB | 500 KB | 361 KB |
| opt-level `"s"`, thin LTO | name section removed | 1,340 KB | 451 KB | 326 KB |
| opt-level `"s"`, thin LTO | `wasm-opt -O3` | 1,259 KB | 464 KB | 337 KB |
| opt-level `"z"`, thin LTO | none | 1,984 KB | 511 KB | 367 KB |
| opt-level `"z"`, thin LTO | **name section removed (smallest)** | **1,321 KB** | **438 KB** | **318 KB** |
| opt-level `"z"`, thin LTO | `wasm-opt -Oz` | 1,221 KB | 458 KB | 332 KB |
| opt-level 3, regex without `perf` (scratch copy) | name section removed | 1,175 KB | 412 KB | 293 KB |

The full 45-row matrix (including `-Os` and fat LTO for every level) is printed by `size.py`.

Findings:

- **The `name` section is the waste.** It holds demangled function names: 199 KB at opt-level 3 and
  664 KB at `"z"` (less inlining, more functions). That is why `"s"`/`"z"` look *bigger* than 3
  before it is removed.
- **Remove it after wasm-bindgen, not in rustc.** `strip = true` in the Cargo profile gives the same
  size, but it also drops the `target_features` section. wasm-bindgen 0.2.130 then warns that it
  cannot detect Wasm features and falls back to a return-pointer ABI (different glue and anonymous
  export names such as `__wbindgen_export2`). It still works, but it is not the normal output.
  `wasm-bindgen --remove-name-section --remove-producers-section` (or the equivalent filter in
  `size.py`) keeps the glue byte-identical to today's.
- **`wasm-opt` needs explicit features on stripped input.** Without `target_features` it rejects
  `memory.copy`; pass `--enable-bulk-memory --enable-bulk-memory-opt
  --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals --enable-multivalue
  --enable-reference-types --enable-call-indirect-overlong` (rustc 1.99's default wasm32 set).
- **wasm-pack with `--profile <custom>` ignores the `wasm-opt = false` metadata** and downloads and
  runs `wasm-opt` anyway. Keep using `--release` or `--no-opt`.

### What the module contains

Per crate, from [twiggy](https://github.com/AlexEne/twiggy) 0.8 on the release module (code only;
the `.rodata` data segment cannot be split by crate):

| Item | Size | Share |
|---|---|---|
| `.rodata` data segment (Unicode and other tables) | 502 KB | 29% |
| regex-automata + regex-syntax + aho-corasick + regex | 518 KB | 30% |
| `name` section | 199 KB | 12% |
| core + alloc + std (generic code, formatting) | 198 KB | 12% |
| serde_json | 77 KB | 4% |
| similar | 68 KB | 4% |
| diffyman-algo + diffyman + diffyman-model | 71 KB | 4% |
| everything else (hashbrown, unicode-segmentation, imara-diff, sha2, dlmalloc, ...) | 74 KB | 4% |

Stand-alone probes (one exported function per crate, opt-level 3, stripped, ~16 KB baseline included):

| Probe | Raw | gzip -9 |
|---|---|---|
| regex 1.13.1, default features | 1,149 KB | 348 KB |
| regex, `std` + `unicode` (no `perf`) | 743 KB | 231 KB |
| regex, `std` + `unicode` + `perf-dfa` + `perf-literal` | 1,073 KB | 332 KB |
| regex, `std` + `perf` (no `unicode`; `\d` no longer compiles) | 833 KB | 255 KB |
| tree-sitter 0.27.2 + tree-sitter-json 0.24.8 | 131 KB | 49 KB |
| similar 3.2.1 (Patience over lines) | 128 KB | 43 KB |
| serde_json 1.0.154 (`Value` round trip) | 100 KB | 41 KB |
| icu_casemap + icu_properties 2.4.0 (fold + Script) | 93 KB | 30 KB |
| unicode-segmentation 1.13.3 (graphemes + word bounds) | 84 KB | 32 KB |
| imara-diff 0.2.0 (Histogram) | 53 KB | 22 KB |
| sha2 0.11.1 (SHA-256) | 25 KB | 10 KB |

regex matching speed in those probes (1 MB text in Node, median of 5; match counts are identical):

| Pattern | default | no `perf` | `perf-literal` only | `perf-dfa` only | `perf-dfa` + `perf-literal` |
|---|---|---|---|---|---|
| `\d+` | 1.2 ms | 27.7 ms | 27.7 ms | 0.8 ms | 0.9 ms |
| `\b(?:alpha\|beta\|gamma)\b` | 3.5 ms | 47.5 ms | 6.8 ms | 42.7 ms | 3.6 ms |
| `(?i)café\|日本` | 2.7 ms | 45.8 ms | 5.4 ms | 2.5 ms | 2.7 ms |

So the only trim that keeps full speed (`perf-dfa` + `perf-literal`) saves just 76 KB raw. Unicode
data for case folding, scripts and segmentation (ICU4X, unicode-segmentation) is small next to regex.

## 2. Speed: Node.js against the native CLI

[`speed.py`](../../bindings/wasm/bench/speed.py) runs the release CLI (`diff --json`, a new process per
run) and each `--target nodejs` package in process
([`node_speed.mjs`](../../bindings/wasm/bench/node_speed.mjs)). One warm-up, then the median of 5 runs.
"Native engine" is `provenance.timing.totalMicros` from the CLI document: decode, diff and `id`, but not
the final JSON serialization, which the WASM timing includes. Every package returned the CLI's document
(with `provenance` removed).

| Surface | 1 MB median | 10 MB median | 1 MB cold call | 10 MB cold call |
|---|---|---|---|---|
| Native engine, in process | **6.0 ms** | **76.0 ms** | n/a | n/a |
| Native CLI, whole process | 9.7 ms | 87.0 ms | n/a | n/a |
| WASM in Node, opt-level 3 (today) | **10.1 ms** | **114.1 ms** | 31.8 ms | 288.2 ms |
| WASM in Node, opt-level 3 + `wasm-opt -O3` | 10.2 ms | 113.0 ms | 32.6 ms | 287.3 ms |
| WASM in Node, opt-level 3, regex without `perf` | 10.2 ms | 113.3 ms | 31.4 ms | 287.4 ms |
| WASM in Node, opt-level `"s"` + `wasm-opt -O3` | 12.5 ms | 133.1 ms | 34.0 ms | 303.5 ms |
| WASM in Node, opt-level `"z"` + `wasm-opt -O3` | 15.0 ms | 173.2 ms | 37.2 ms | 350.8 ms |

- WASM is **1.7 times** (1 MB) and **1.5 times** (10 MB) slower than the native engine, in line with
  Spike B (1.6 times on 100k lines).
- The **first call is about 3 times slower** in V8 (baseline compiler first, optimized code later).
  Loading the package (`require`, which compiles the module) takes 2 to 3 ms.
- The 10 MB pair needs no special handling in WASM; memory was not measured.

## 3. Browsers

[`bench/index.html`](../../bindings/wasm/bench/index.html) loads `../pkg-web` (today's 1,705 KB
module), reads the same inputs with synchronous XHR, calls `initSync`, makes one cold call and 5 timed
calls, and writes the result into the DOM before the load event. It is served by
[`serve.py`](../../bindings/wasm/bench/serve.py) (package at `/`, inputs at `/data/`) because the page
reads its inputs over HTTP.

| Browser | Command | `initSync` | 1 MB cold | 1 MB median | 10 MB cold | 10 MB median |
|---|---|---|---|---|---|---|
| Chrome 157 | `--headless=new --disable-gpu --dump-dom URL` | 1.3 to 2.9 ms | 32.3 ms | **11.4 ms** | 293.6 ms | **121.4 ms** |
| Firefox 157.0.1 | `--headless --screenshot out.png URL` | 6 ms | 21 ms | **12 ms** | 176 ms | **134 ms** |
| Safari | not measured | | | | | |

- Both browsers return the same hunk count and document length as Node (115 and 1,189 hunks).
- Timers are coarse without cross-origin isolation (Chrome 0.1 ms, Firefox 1 ms), which is fine at
  these durations.
- Firefox runs headless with a throwaway profile (`--no-remote --profile DIR`); the result is read
  from the screenshot. Its first call is cheaper than V8's (1.3 to 1.8 times the median, not 3).
- **Safari** has no headless mode. `safaridriver` needs "Allow Remote Automation" switched on by hand
  once, so it was left out. Playwright's WebKit build is the scriptable alternative (follow-up).
- A real page should call the async `init()` (streaming compile) and run diffs in a Worker. The
  synchronous path here only exists so a headless browser can dump the result without waiting.

## 4. Planned crates on `wasm32-unknown-unknown`

A scratch workspace under `~/diffyman-data/tmp` (outside the repo) has one crate per dependency, each
using a real API of that crate, built with the pinned toolchain. Versions are the planned pins; all
resolved. zip 9.4.2 is out; 9.4.1 was used as pinned.

| Crate | Version | `cargo check` | `cargo build` (cdylib) | Run in Node | Notes |
|---|---|---|---|---|---|
| similar | 3.2.1 | pass | pass | yes | Already in the engine |
| imara-diff | 0.2.0 | pass | pass | yes | Already in the engine |
| unicode-segmentation | 1.13.3 | pass | pass | yes | Already in the engine |
| regex | 1.13.1 | pass | pass | yes | Largest dependency (see §1) |
| serde_json | 1.0.154 | pass | pass | yes | Already in the engine |
| sha2 | 0.11.1 | pass | pass | yes | Already in the engine |
| encoding_rs | 0.8.44 | pass | pass | no | Pure Rust |
| tree-sitter | 0.27.2 | pass | pass | yes, with tree-sitter-json 0.24.8 | C code; see below |
| image (`default-features = false`) | 0.25.10 | pass | pass | no | Also passes with `png` + `jpeg` |
| calamine | 0.38.0 | pass | pass | no | Uses zip with `deflate` only (pure Rust) |
| zip (default features) | 9.4.1 | pass | pass | needs wasm-bindgen glue | Pulls `zstd-sys` (C), `getrandom` 0.4 and `time` with `wasm-bindgen`/`js-sys` imports |
| zip (`default-features = false`, `deflate`) | 9.4.1 | pass | pass | no | Pure Rust |
| quick-xml | 0.42.0 | pass | pass | no | Pure Rust |
| hayro | 0.9.0 | pass | pass | no | Pure Rust (pulls image, vello_cpu, a brotli decoder) |

"Run in Node" means the module was instantiated with no imports and its function returned the expected
value. A pass on `check`/`build` does not prove the code works at run time (for example, a crate that
calls `std::time::Instant::now()` panics on this target).

**tree-sitter needed two fixes, both build-environment only:**

1. **Archiver.** cc-rs uses the system `ar` for the wasm32 objects. On macOS, Apple's `ar` writes a
   symbol index `wasm-ld` cannot read, so linking fails with `undefined symbol: ts_parser_new`. Setting
   `AR_wasm32_unknown_unknown` to LLVM's `llvm-ar` fixes it. The C compiler itself is fine: Apple clang
   21 has the WebAssembly backend.
2. **Grammar headers.** tree-sitter 0.27 compiles its own small libc for `wasm32-unknown-unknown` and
   publishes the headers through `tree-sitter-language`. Grammar crates made with older tree-sitter CLI
   versions (tree-sitter-json 0.24.8) do not add them and fail with `'stdlib.h' file not found`. Adding
   `-I <tree-sitter-language>/wasm/include` through `CFLAGS_wasm32_unknown_unknown` fixes it. The
   module then has no imports and parses JSON correctly in Node.

## Problems and follow-ups

1. **Package with wasm-bindgen directly for releases.** wasm-pack 0.15.1 cannot pass
   `--remove-name-section`. The release job should run `cargo build --release --target
   wasm32-unknown-unknown -p diffyman-wasm` and `wasm-bindgen --target web|nodejs
   --remove-name-section --remove-producers-section` with `wasm-bindgen-cli` pinned to the same
   version as the crate (0.2.130). wasm-pack stays fine for local development.
2. **regex.** It is about half the module but unused until `ignore.regex` ships. Options: make regex an
   optional feature of `diffyman-algo` and leave it out of the WASM build until then, or drop its
   `perf` features in the WASM build (saves 79 KB brotli, 14 to 24 times slower matching). Decide
   when `ignore.regex` is implemented; `unicode` stays on in every build so patterns mean the same
   thing everywhere.
3. **Size check in CI.** Run `size.py` (or a smaller check) on the release package and fail on a large
   unexplained jump. The target hard gate is 8 MiB raw ([ADR 0008](../adr/0008-wasm-target.md)); the recommended build is 18% of that.
4. **C dependencies in CI.** Linux runners need `clang` and `llvm-ar` for tree-sitter (and zstd if it
   is ever enabled). Vendored tree-sitter grammars need a `build.rs` that adds the wasm headers.
5. **zip.** Use `default-features = false` with explicit features, so the WASM build (and the planned
   `wasm32-wasip1` build for Go and Java) gets no `getrandom`/`time` JS imports and no C.
6. **Safari, and memory.** Measure Safari through Playwright WebKit or `safaridriver`, and record peak
   memory for the 10 MB pair (and a 100 MB pair) in all three browsers before the browser demo.
7. **License files.** Still open from Spike B: wasm-pack warns that `bindings/wasm` has no `LICENSE*`
   files.

## Reproduce

From the repository root (prefix Rust tools with `scripts/with-rustup` if another Rust comes first on
`PATH`). `wasm-opt` is only needed for the comparison rows.

```sh
npm install --prefix ~/diffyman-data/tmp/binaryen binaryen@132.0.0
uv run --no-project --python 3.12 --with brotli==1.1.0 \
    python bindings/wasm/bench/size.py --wasm-opt ~/diffyman-data/tmp/binaryen/node_modules/.bin/wasm-opt

uv run --no-project --python 3.12 python bindings/wasm/bench/gen_inputs.py
cargo build --release -p diffyman-cli
wasm-pack build bindings/wasm --release --target nodejs --out-dir pkg-node
uv run --no-project --python 3.12 python bindings/wasm/bench/speed.py \
    --cli target/release/diffyman --pkg release=bindings/wasm/pkg-node

wasm-pack build bindings/wasm --release --target web --out-dir pkg-web
uv run --no-project --python 3.12 python bindings/wasm/bench/serve.py &
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --disable-gpu \
    --dump-dom "http://127.0.0.1:8765/bench/index.html?size=1"
/Applications/Firefox.app/Contents/MacOS/firefox --headless --no-remote --profile "$(mktemp -d)" \
    --window-size=1100,1500 --screenshot firefox.png "http://127.0.0.1:8765/bench/index.html?size=10"
```

For the opt-level rows of the speed table, build extra `--target nodejs` packages with
`wasm-pack build ... --no-opt --out-dir DIR -- --config 'profile.release.opt-level="s"'` and pass
each as another `--pkg NAME=DIR`.
