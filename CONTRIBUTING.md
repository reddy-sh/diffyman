# Contributing to diffyman

Thanks for your interest. diffyman is early: the Rust workspace, the Diff Document spec and the golden corpus are taking shape, so issues and small pull requests are the easiest way to help.

## Ground rules

- **The spec comes first.** The Diff Document is defined in [`docs/spec/`](docs/spec/). A change to the output shape needs a spec change (and, for schema changes, an ADR in [`docs/adr/`](docs/adr/)) in the same pull request.
- **Same answer everywhere.** Output must be deterministic: no hash-map order, no locale, no timing outside `provenance`. Any change that alters the diff produced for some input bumps the algorithm ID (for example `text.histogram@1` to `@2`) and regenerates the affected `expected.json` files.
- **Dependencies** are pinned to exact versions and must pass the license allowlist in [`deny.toml`](deny.toml) (no GPL, AGPL or LGPL).
- Be kind. See the [Code of Conduct](CODE_OF_CONDUCT.md).

## Sign your commits (DCO)

Contributions are accepted under the [Developer Certificate of Origin](https://developercertificate.org/). Add a `Signed-off-by` line to every commit to certify that you wrote the change or have the right to submit it under the project's licenses:

```sh
git commit -s -m "Describe the change"
```

This adds `Signed-off-by: Your Name <you@example.com>` using your git `user.name` and `user.email`. Unless you state otherwise, contributions are dual licensed under MIT OR Apache-2.0, as described in the [README](README.md#license).

## Build and test

The toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml); rustup installs it on first use. From the repository root:

```sh
cargo build --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p diffyman-conformance      # golden corpus
cargo deny check                        # licenses, advisories, bans, sources
```

If another Rust installation (for example a system package) comes before rustup's proxies on your `PATH`, `rust-toolchain.toml` is ignored. Run the same commands through the wrapper, which puts rustup first:

```sh
scripts/with-rustup cargo test --workspace
```

Try the CLI:

```sh
cargo run -p diffyman-cli -- diff a.txt b.txt            # unified-style text diff
cargo run -p diffyman-cli -- diff a.txt b.txt --json     # canonical JSON Diff Document
cargo run -p diffyman-cli -- schema                      # JSON Schema of the document
```

Exit codes: `0` identical, `1` different, `2` error.

## Workspace layout

| Crate | Role |
|---|---|
| `crates/diffyman-model` | Diff Document types, RFC 8785 canonical JSON, `id`, JSON Schema |
| `crates/diffyman-algo` | Line splitting, comparison keys, line diff, hunks, inline refinement, moved blocks |
| `crates/diffyman-engine` (package `diffyman`) | Public API: bytes in, canonical JSON out |
| `crates/diffyman-cli` | The `diffyman` binary |
| `crates/diffyman-conformance` | Golden-corpus runner |
| `bindings/python` | Python package `diffyman` (PyO3 + maturin, abi3 for CPython 3.11+) |
| `bindings/wasm` | WebAssembly package (wasm-bindgen + wasm-pack) |

## Bindings

Bindings are thin: bytes and an options JSON string go in, the engine's canonical JSON comes out ([ADR 0007](docs/adr/0007-binding-strategy.md)). `tests/conformance/run_bindings.py` checks that the CLI, the Python binding and the WASM binding return byte-identical documents for every corpus case. It needs [uv](https://docs.astral.sh/uv/), Node.js 22+, maturin 1.16.0 and wasm-pack 0.15.1:

```sh
cargo build --release -p diffyman-cli
uv venv --python 3.12 bindings/python/.venv
uv pip install --python bindings/python/.venv/bin/python maturin==1.16.0 pytest==9.2.0
VIRTUAL_ENV=$PWD/bindings/python/.venv bindings/python/.venv/bin/maturin develop --uv --release -m bindings/python/Cargo.toml
bindings/python/.venv/bin/python -m pytest -q bindings/python/tests
wasm-pack build bindings/wasm --release --target nodejs --out-dir pkg-node
uv run --no-project --python 3.12 python tests/conformance/run_bindings.py
```

## Golden corpus

Cases live in `tests/corpus/text/<case>/` and `tests/corpus/moved/<case>/`. Each holds `left.*`, `right.*`, `options.json` and `expected.json` (moved cases also have `labels.json`). `expected.json` is the canonical JSON of the document without `provenance`, written as exact bytes with no trailing newline.

```sh
cargo run -p diffyman-conformance              # compare; cases without expected.json are reported as NEW
cargo run -p diffyman-conformance -- --bless   # write expected.json for every case
cargo run -p diffyman-conformance -- --strict  # NEW cases fail
```

Review every blessed file before committing it. Corpus files are stored byte for byte (see `.gitattributes`), so line endings and BOMs in them are part of the test. Every case must be original material: never copy another product's output.

## Pull requests

- Keep each pull request focused, with tests for new behavior.
- Make sure format, clippy, tests, the conformance run and `cargo deny check` pass locally; CI runs the same checks on Linux, macOS and Windows.
- Describe what changed and why. Link the spec section or issue it relates to.
