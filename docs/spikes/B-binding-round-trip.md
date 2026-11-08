# Spike B: binding round trip (CLI, Python, WASM)

- **Date:** 2026-11-07
- **Question:** Do thin bindings return the same canonical Diff Document as the Rust CLI, byte for byte,
  for every corpus case? Python and WASM are required for the Phase 0 gate; Node (napi-rs) and Java (JNI)
  are optional for this spike.
- **Code:** [`bindings/python`](../../bindings/python), [`bindings/wasm`](../../bindings/wasm),
  [`tests/conformance/run_bindings.py`](../../tests/conformance/run_bindings.py),
  [`tests/conformance/wasm_diff.mjs`](../../tests/conformance/wasm_diff.mjs),
  [`tests/conformance/jcs.py`](../../tests/conformance/jcs.py)
- **Decision:** [ADR 0007](../adr/0007-binding-strategy.md)

## Summary

1. **The gate passes.** For all 60 corpus cases (20 `text`, 40 `moved`), the CLI, the Python binding and
   the WASM binding return documents whose RFC 8785 bytes, with `provenance` removed, are identical to each
   other and to `expected.json`.
2. **40 more real diffs agree.** Move detection is deferred in schema 0.1.0, so every `moved` case is an
   `unsupported-option` error document. The runner therefore also diffs the 40 moved input pairs with
   `"detectMoves": false`. Those 40 line diffs (CRLF, CJK, right-to-left text, indentation changes) are
   byte-identical across the three surfaces too. They have no golden file yet.
3. **Every surface emits canonical JSON as is.** Re-canonicalizing each raw output with an independent
   Python JCS implementation leaves it unchanged, so the Rust canonicalizer is cross-checked on every run.
4. **The bindings hold no diff logic.** Each is about 60 lines of Rust over one new engine entry point,
   `diffyman::diff_inputs_with(left, right, options_json, settings)`. The only binding-specific output is
   `provenance.binding` (`cli`, `python`, `wasm`).
5. **One abi3 wheel covers CPython 3.11 to 3.15.** The same `cp311-abi3` wheel passes the 80 pytest
   tests on 3.11.16, 3.12.12 and 3.15.0.
6. **Node (napi-rs) and Java (JNI) were not attempted.** Both are optional here, and neither is trivial:
   napi-rs needs its own npm CLI and per-platform packages, JNI needs a Java build and native loading. Both
   will use the same JSON-in/JSON-out entry point (ADR 0007).

## What was built

| Surface | Mechanism (exact pins) | Entry point |
|---|---|---|
| CLI | `diffyman diff --json --options FILE LEFT RIGHT` | existing |
| Python | PyO3 `=0.30.1` (feature `abi3-py311`), maturin 1.16.0, module `diffyman._native` | `diffyman.diff(left, right, options=None, *, left_name=None, right_name=None) -> str`, `diffyman.diff_document(...) -> dict` |
| WASM | wasm-bindgen `=0.2.130`, wasm-pack 0.15.1, `--target nodejs` (`pkg-node/`) and `--target web` (`pkg-web/`) | `diff(left: Uint8Array, right: Uint8Array, optionsJson: string, leftName?: string, rightName?: string): string` |

Details that matter for conformance:

- **Input names are part of `id`.** The required WASM signature has no names, so `leftName` and
  `rightName` are optional trailing arguments. Python takes the name from an `os.PathLike` input or from
  `left_name`/`right_name`. The harness passes the corpus file names to every surface.
- **`str` is text, not a path,** in Python. Paths must be `os.PathLike` (for example `pathlib.Path`).
  Guessing would make the same call mean different things depending on the file system.
- **Errors.** Invalid options produce no document. Python raises `diffyman.DiffymanError` (a
  `ValueError`); WASM throws an `Error`. Everything else (invalid UTF-8, unsupported options) is a document
  with `status: "error"`, as the spec requires.
- **Python releases the GIL** (`Python::detach`) while the engine runs.
- **wasm32.** The engine already gated its clock for `wasm32-unknown-unknown` (`totalMicros` is 0 there).
  No threads, no file system, no mmap. One fix: `std::env::consts::OS` is empty on that target, so
  `provenance.platform.os` now reads `unknown` instead of `""`. This changes provenance only.

## Results

`uv run --no-project --python 3.12 python tests/conformance/run_bindings.py` on the development Mac
(Apple M4 Max, macOS 27.0, rustc 1.99.1, Node v22.23.4, CPython 3.12.12):

| Group | Cases | Statuses | CLI | Python | WASM | All three identical | Equal to `expected.json` |
|---|---|---|---|---|---|---|---|
| `text` | 20 | 3 identical, 16 different, 1 error | 20 | 20 | 20 | 20 | 20 |
| `moved` | 40 | 40 error (`detectMoves` deferred) | 40 | 40 | 40 | 40 | 40 |
| `moved` with `detectMoves: false` | 40 | 40 different | 40 | 40 | 40 | 40 | n/a (no golden file) |

The runner also checks that the CLI exit code matches the status (0, 1, 2), and it fails when any
surface differs. Editing one field of one `expected.json` turns that case into `FAIL` on all three
surfaces, as expected.

Other checks:

- `cargo fmt --check`, `cargo clippy --workspace --all-targets [--all-features] -- -D warnings`,
  `cargo test --workspace` (91 tests) and the golden-corpus runner (60/60) pass.
  `cargo build --workspace` links (the Python extension is built without `extension-module` there).
- `cargo deny check` passes with the new dependencies (PyO3, wasm-bindgen and their dependencies are all
  MIT or Apache-2.0).
- `cargo build -p diffyman -p diffyman-wasm --target wasm32-unknown-unknown` passes.
- pytest: 80 tests (all 60 corpus cases plus input kinds, names, option forms, errors and threads).
- The `--target web` package loads in Node from bytes (`init({ module_or_path })`) and gives the same
  result.

## Measurements

Indicative only (single machine, median of 5 runs). Input: 100,000 lines with 100 changed lines, giving
a 41 KB document.

| Surface | 100k-line diff | Tiny diff (2 lines) |
|---|---|---|
| Python, in process | 28.5 ms | 16.6 µs per call |
| WASM in Node, in process | 45.9 ms | 25.6 µs per call |
| CLI, new process each time | 35.8 ms | n/a |

| Artifact | Size | gzip -9 |
|---|---|---|
| `diffyman_wasm_bg.wasm` (no `wasm-opt`) | 1.71 MB | 550 KB |
| Python wheel (`cp311-abi3`, macOS arm64) | 990 KB | n/a (already compressed) |
| `diffyman` CLI (release, macOS arm64) | 3.05 MB | 1.19 MB |

WASM is about 1.6 times slower than native here, which is acceptable for the browser and edge paths.
Size work (`wasm-opt`, Unicode data tables, feature trimming) belongs to Spike C.

## CI

`.github/workflows/ci.yml` gains a `bindings` job on `ubuntu-latest` and `macos-latest`. It installs the
pinned toolchain, sets up uv (`astral-sh/setup-uv` v10.3.3) and Node 22 (`actions/setup-node` v7.1.0),
both pinned by commit SHA. It then installs wasm-pack 0.15.1, builds the CLI, builds the Python binding
with `maturin develop`, runs pytest, builds both WASM packages and runs `run_bindings.py`. The job has
not run on GitHub yet. Windows is left out of this job for now; the `check` job still covers Windows
for the Rust crates.

## Problems and follow-ups

1. **WASM size.** 1.71 MB unoptimized. `wasm-opt` is turned off so the build needs no extra download.
   Spike C measures `wasm-opt -Oz` and which Unicode data dominates.
2. **License files for npm.** wasm-pack warns that the crate folder has no `LICENSE*` files. Copy both
   license files into `bindings/wasm` (or generate them at packaging time) before the first npm release.
3. **Typed wrappers.** Both bindings return the canonical string plus a parsed `dict` or JSON value.
   Typed views (Python dataclasses, TypeScript types) should be generated from the JSON Schema
   (`diffyman schema`), not written by hand, so they cannot drift. Planned for Phase 1.
4. **Golden files for line-only moved inputs.** The 40 extra cases are checked only across surfaces.
   When move detection ships, the moved cases get real `moves[]` and these extra cases can be dropped
   or given their own golden files.
5. **Node and Java.** napi-rs (`@diffyman/core`) and JNI join `run_bindings.py` as two more columns
   when they are built.

## Reproduce

From the repository root, with uv, Node 22+, maturin 1.16.0 and wasm-pack 0.15.1 installed
(prefix Rust tools with `scripts/with-rustup` if another Rust comes first on `PATH`):

```sh
cargo build --release -p diffyman-cli
uv venv --python 3.12 bindings/python/.venv
uv pip install --python bindings/python/.venv/bin/python pytest==9.2.0
VIRTUAL_ENV=$PWD/bindings/python/.venv maturin develop --uv --release -m bindings/python/Cargo.toml
bindings/python/.venv/bin/python -m pytest -q bindings/python/tests
wasm-pack build bindings/wasm --release --target nodejs --out-dir pkg-node
wasm-pack build bindings/wasm --release --target web --out-dir pkg-web
uv run --no-project --python 3.12 python tests/conformance/run_bindings.py
```
