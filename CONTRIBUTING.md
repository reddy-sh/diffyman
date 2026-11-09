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
| `crates/diffyman-storage` | Content-addressed blob store over `object_store` (`file://`, `memory://`, `s3://`) |
| `crates/diffyman-db` | PostgreSQL schema, migrations and queries (`sqlx`, offline data in `.sqlx/`) |
| `crates/diffyman-server` | The `diffyman-server` binary: the local REST service ([guide](docs/server.md), [API](docs/api/README.md)) |

Only the three server crates use a database, the network or an async runtime; the engine, the CLI and the bindings must keep building without them.

## Bindings

Bindings are thin: bytes and an options JSON string go in, the engine's canonical JSON comes out ([ADR 0007](docs/adr/0007-binding-strategy.md)). `tests/conformance/run_bindings.py` checks in one run that the CLI, the Python binding, the WASM binding (with and without archive support, [ADR 0008](docs/adr/0008-wasm-target.md)), the Node.js binding, the Java binding and the C ABI return byte-identical documents for every corpus case, equal to `expected.json` with `provenance` removed. [tests/conformance/README.md](tests/conformance/README.md) lists the build commands for every surface and the runner's flags (`--surfaces`, `--skip`, `--hashes`):

```sh
uv run --no-project --python 3.12 python tests/conformance/run_bindings.py --cli target/release/diffyman
```

CI runs it on Linux, macOS and Windows and compares the per-case SHA-256 tables of the three runs, so the documents are also identical across operating systems.

## Server tests

`cargo test --workspace` passes without a database: the tests of `diffyman-db`, `diffyman-storage` and `diffyman-server` that need PostgreSQL print why they skip and pass, so a machine without Docker can still run everything else. To run them all, start the local PostgreSQL 18 on `127.0.0.1:5433` (it writes a random password to the gitignored `.env.local`, which the tests read):

```sh
scripts/local-up.sh
cargo test -p diffyman-db -p diffyman-storage -p diffyman-server
cargo test -p diffyman-server -- --nocapture 2>&1 | grep skipping   # nothing printed: nothing skipped
```

- Every database test runs in a fresh schema of its own in a `diffyman_test` database; the main `diffyman` database is never used. `DIFFYMAN_TEST_DB_URL` points the tests at another PostgreSQL server (a URL to any database on it, usually `postgres`). `DIFFYMAN_TEST_DB_REQUIRED=1` turns a skip into a failure, as in CI.
- Builds use the offline query data in `.sqlx/`, so they need no database (`SQLX_OFFLINE=true` in CI). After changing a migration or a query in `crates/diffyman-db`, refresh it with `scripts/db-prepare.sh` (it needs `sqlx-cli` 0.9.0; the script prints the install command) and commit `.sqlx/`; `scripts/db-prepare.sh --check` is what CI runs.
- When the API changes, `cargo test -p diffyman-server --test openapi` rewrites `docs/api/openapi.json` and fails once; commit the new file.

The built binary has its own checks, each on its own database, blob folder and random port on `127.0.0.1` (they need `psql`, and `curl` and `jq` for the last one):

```sh
cargo build --locked --release -p diffyman-server -p diffyman-cli
uv run --no-project --python 3.12 python tests/server/run_e2e.py \
  --server target/release/diffyman-server --cli target/release/diffyman   # end to end, about 1 GB free disk
uv run --no-project --python 3.12 python tests/server/harness.py -- \
  python tests/conformance/run_bindings.py --surfaces cli,server --cli target/release/diffyman
uv run --no-project --python 3.12 python docs/api/check_examples.py      # the curl examples of the docs
```

[tests/server/README.md](tests/server/README.md) describes the end-to-end run and its options, including a run against the S3-compatible server of the `s3` profile. Keep secrets out of command lines and logs: the scripts read the database password from `.env.local` and pass it to child processes only through their environment, and the server never logs file content, file paths or keys.

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
