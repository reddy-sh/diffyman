# 0001. Rust for the core engine

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

diffyman is a headless engine that diffs any file and returns one JSON result. The same engine must run as a CLI, inside Python, Node, Java and browser processes, behind a REST API, and later inside desktop and mobile apps. It will open PDFs, archives, spreadsheets and images from strangers, so its parsers are an attack surface. It must also handle GB-scale inputs within a fixed time and memory budget.

## Decision

Write the core in **Rust**, as a Cargo workspace of small crates (`diffyman-model`, `diffyman-algo`, the adapter crates, `diffyman-engine`, `diffyman-cli`, `diffyman-ffi` and the server-side crates). Pin the toolchain in `rust-toolchain.toml`.

Reasons:

- **Safe parsing of untrusted files.** Memory safety by default, with `unsafe` confined to reviewed binding and FFI code.
- **One core for every target.** `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` and the Windows MSVC targets are Tier 1. The `wasm32-unknown-unknown` and `wasm32-wasip*` targets are Tier 2.
- **Binding generators exist for almost every language:** PyO3, napi-rs, wasm-bindgen, jni, UniFFI, cbindgen, csbindgen, magnus, rustler, ext-php-rs and flutter_rust_bridge.
- **Proven pattern.** polars, pydantic-core, tokenizers, cryptography and orjson ship a Rust core to Python this way.
- **Building blocks already exist:** `similar` and `imara-diff` (line diff), `tree-sitter` (parsing), with `difftastic` (MIT) as a structural-diff reference.
- **Cloud fit later.** It builds to a small static binary for containers or short-lived functions.

Alternatives rejected:

| Option | Why not |
|---|---|
| C / C++ | Fast, but unsafe when parsing hostile files, and cross-platform builds are painful. |
| Go | Other languages would call it through cgo with a Go runtime inside, and the WASM story is weaker. Go stays a binding target. |
| TypeScript / Node | Slow on large files, and Python, Java and other hosts would need an embedded JS engine. |
| Java / Kotlin | Needs a JVM everywhere, which makes the CLI, WASM and Python embedding awkward. |
| Zig | Pre-1.0, with a small ecosystem of format libraries. |
| Haxe | Cross-compiles to many languages, but the ecosystem is small. |
| Swift | The non-Apple and WASM story is weaker. |

## Consequences

- Every surface (CLI, bindings, REST API) is a thin layer over `diffyman-engine`. Diff logic is written once (see ADR 0002).
- Contributors need Rust. Binding authors also need the host language's build tool (maturin, napi-rs CLI, wasm-pack, a JDK).
- CI must build a target matrix across three operating systems plus WASM.
- Some crates may not compile to `wasm32`. Spike C (ADR 0008) checks this. Features that cannot (for example a PDFium-backed renderer or the plugin host) stay behind cargo features that the WASM and mobile builds leave out.
- The line diff library choice (`similar` or `imara-diff`) is left to spike A (ADR 0006). Either way it sits behind our own versioned algorithm IDs, so it can be swapped later.
- We pin exact crate versions, because some diff libraries treat any change in produced output as a breaking change.
