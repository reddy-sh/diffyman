# Cross-binding conformance

`run_bindings.py` runs every corpus case (`tests/corpus/*/*`) through the CLI
and every binding and checks that each surface returns the same canonical
Diff Document, byte for byte, once the top-level `provenance` key is removed,
and that it equals `expected.json` ([spec §3](../../docs/spec/diff-document.md),
[ADR 0007](../../docs/adr/0007-binding-strategy.md)). Each output must also be
RFC 8785 canonical as written (checked with the independent `jcs.py`) and name
its own surface in `provenance.binding`.

| Surface | How it runs | Modes |
|---|---|---|
| `cli` | `diffyman diff --json --options ...`, one process per case; the exit code must match the status | paths |
| `python` | `diffyman.diff()` in the binding's virtual environment (`surfaces.py --python-worker`) | paths, bytes |
| `wasm` | the `--target nodejs` package with default features (`wasm_diff.mjs`) | bytes |
| `wasm-slim` | the same package built without `archives`: must refuse the cases of spec §19.7 with an `unsupported` error | bytes |
| `node` | `diffPaths()` and `diff()` of `bindings/node` (`node_diff.mjs`) | paths, bytes |
| `java` | `Diffyman.diffPaths()` and `Diffyman.diff()` from the binding jar (`JavaDiff.java`) | paths, bytes |
| `c` | `diffyman_diff_paths()` and `diffyman_diff()`: the C test program built by `tests/ffi/run.sh`, or the shared library through `ctypes` (`--c-driver`) | paths, bytes |
| `server` | `POST /v1/diff` of a running `diffyman-server` (`--server-url` or `DIFFYMAN_SERVER_URL`, API key in `DIFFYMAN_SERVER_KEY`); skipped with a note when no URL is given | json (base64), multipart |

In `paths` mode the surface reads the inputs itself; in `bytes` mode two
files are read first and passed as bytes with their final path components as
names. A case a surface cannot run is skipped with its reason, and the summary
counts each reason: folder inputs have no `bytes` form, and the WASM binding
takes bytes only. The moved cases also run with `"detectMoves": false`
(no golden file; the surfaces must agree), unless `--golden-only` is given.

## Running it

Build every surface from the repository root, then run the checker (it needs
[uv](https://docs.astral.sh/uv/), Node.js 22+, a JDK 17+ with Maven, a C
compiler, maturin 1.16.0 and wasm-pack 0.15.1):

```sh
cargo build --locked --release -p diffyman-cli -p diffyman-ffi -p diffyman-java
uv venv --python 3.12 bindings/python/.venv
VIRTUAL_ENV=$PWD/bindings/python/.venv uv pip install maturin==1.16.0 pytest==9.2.0
VIRTUAL_ENV=$PWD/bindings/python/.venv bindings/python/.venv/bin/maturin develop --uv --release --locked -m bindings/python/Cargo.toml
wasm-pack build bindings/wasm --release --target nodejs --out-dir pkg-node -- --locked
wasm-pack build bindings/wasm --release --target nodejs --out-dir pkg-node-slim -- --locked --no-default-features
npm --prefix bindings/node ci && npm --prefix bindings/node run build
mvn -B -f bindings/java/pom.xml verify
PROFILE=release PYTHON=bindings/python/.venv/bin/python bash tests/ffi/run.sh

uv run --no-project --python 3.12 python tests/conformance/run_bindings.py --cli target/release/diffyman
```

Useful flags:

- `--surfaces cli,node,c` runs only those surfaces, `--skip java` leaves one
  out. A selected surface that is not built stops the run with the command
  that builds it.
- `--c-driver program|ctypes` picks how the C ABI is reached (default: the
  compiled program when it exists, `ctypes` on Windows, where `run.sh` does
  not run).
- `--server-url http://127.0.0.1:PORT` adds the server surface. To start a
  server on a test database for one run (it needs PostgreSQL, see
  [`tests/server`](../server/README.md)):

  ```sh
  cargo build --locked --release -p diffyman-server -p diffyman-cli
  uv run --no-project --python 3.12 python tests/server/harness.py -- \
    python tests/conformance/run_bindings.py --surfaces cli,server --cli target/release/diffyman
  ```
- `--quiet` prints only failing cases and the summary.
- `--hashes FILE` writes a tab-separated table with the SHA-256 of every
  document (provenance removed) per case, surface and mode, plus that of
  `expected.json`.

## Byte-identical across operating systems

The console table shows the first 16 hex digits of each case's agreed
SHA-256. In CI every runner (Linux, macOS, Windows) uploads its `--hashes`
table as the artifact `conformance-hashes-<os>`, and the job
`bindings-same-everywhere` checks with `compare_hashes.py` that every table
covers the same cases and that each case has exactly one hash across all
surfaces and platforms:

```sh
python3 tests/conformance/compare_hashes.py linux.tsv macos.tsv windows.tsv
```
