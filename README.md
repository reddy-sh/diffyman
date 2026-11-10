# diffyman

**Diff any file, from any language.**

diffyman is an open-source, headless diff engine. One Rust core compares two files or folders and always returns the same versioned JSON result, the **Diff Document**. You get that result from a command-line tool, from language bindings (Python, Java, JavaScript/WASM and more), and from a local REST API.

> **Status:** early development — the engine compares text, binary files, folders and archives, with a CLI, bindings for Python, Node.js, Java and the browser (WASM and React), a C ABI, and a local REST service. Nothing is published to a package registry yet; see [Usage](#usage) and [Use it from your language](#use-it-from-your-language) to build from source, and [docs/STATUS.md](docs/STATUS.md).

**Documentation:** <https://reddy-sh.github.io/diffyman> (coming soon: the site is built and link-checked in CI but not published yet). Until then, read the same pages here: the quickstarts and references linked below, and everything under [docs/](docs/).

## Why

- **Same answer everywhere.** Byte-identical JSON from the CLI, every binding and the API, checked in CI.
- **Local-first.** Files never leave your machine. No telemetry.
- **Any file.** Text and code first, then structured data, spreadsheets, images, PDFs, Office documents, archives and binaries. Folders and archives are compared recursively, and anything else falls back to a hex and metadata diff, so no file is "unsupported".
- **Smarter diffs.** Moved-block detection, semantic JSON/YAML/XML, structural code diff and 3-way merge.
- **Embeddable.** A library first, with stable JSON output and exit codes, git difftool/mergetool integration and an open Diff Document schema.
- **Light and fast.** Native Rust, streaming for very large files, small installs.

## Usage

Nothing is published to a package registry yet. The command-line tool builds from source with a Rust toolchain:

```sh
git clone https://github.com/reddy-sh/diffyman
cd diffyman
cargo install --path crates/diffyman-cli
```

```sh
diffyman diff old.txt new.txt                 # unified diff; exit code 0 identical, 1 different, 2 error
diffyman diff old.txt new.txt -f summary      # verdict and stats
diffyman diff old.txt new.txt --json          # the Diff Document (canonical JSON)
diffyman diff old.md new.md -f html -o report.html
diffyman diff before/ after/                  # folders, or two zip/tar archives
diffyman diff a.log b.log --ignore-whitespace change --ignore-regex '\d+ms'
diffyman git install                          # prints the git difftool config; --global writes it
```

See the [command-line reference](docs/cli.md) for every command, flag and output format.

## Use it from your language

Every surface returns the same Diff Document, byte for byte apart from `provenance`. Each quickstart builds from source, diffs two files, prints the status and the changed lines, and handles errors; [`docs/quickstart/check_examples.py`](docs/quickstart/check_examples.py) runs every example in them against local builds, and [`docs/api/check_examples.py`](docs/api/check_examples.py) runs the REST examples against a real server.

| Language | Quickstart | Package (not published yet) | Reference |
|---|---|---|---|
| Command line | [CLI](docs/quickstart/cli.md) | crate `diffyman-cli` | [CLI reference](docs/cli.md) |
| Python 3.11+ | [Python](docs/quickstart/python.md) | `diffyman` (PyO3, abi3 wheel) | [bindings/python](bindings/python/README.md) |
| Node.js 22+ | [Node.js](docs/quickstart/node.md) | `@diffyman/node` (Node-API) | [bindings/node](bindings/node/README.md) |
| Java 17+ | [Java](docs/quickstart/java.md) | `sh.reddy.diffyman:diffyman` (JNI) | [bindings/java](bindings/java/README.md) |
| Browser, React | [Browser and React](docs/quickstart/browser-react.md) | `@diffyman/react` over `@diffyman/wasm` | [bindings/wasm/react](bindings/wasm/react/README.md), [bindings/wasm](bindings/wasm/README.md), [example app](examples/amplify-react) |
| C, and Go, C#, Zig, Swift through C | [C](docs/quickstart/c.md) | `diffyman-ffi` (shared or static library) | [crates/diffyman-ffi](crates/diffyman-ffi/README.md) |
| Any language over HTTP | [REST API](docs/quickstart/rest.md) | crate `diffyman-server` (a local service) | [API reference](docs/api/README.md), [OpenAPI 3.1](docs/api/openapi.json), [server guide](docs/server.md) |

The document's fields, options and warnings are defined in the [Diff Document specification](docs/spec/diff-document.md).

## Roadmap

| Milestone | Scope |
|---|---|
| **MVP** | Text and code, folders, type detection, hex + metadata fallback. CLI. Rust, Python, Node, browser WASM and Java bindings. Local REST service. Diff Document 1.0-beta |
| **v1** | Semantic JSON/YAML/TOML/XML, structural code, CSV/Excel, images, basic PDF, ZIP/tar, 3-way merge, sandboxed format plugins |
| **v2** | Office documents, rich PDF, notebooks, Markdown, SVG, SQLite/Parquet, executables, API contracts |
| **Later** | Audio/video, 3D/CAD/geo, disk images, hosted service |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

The exception is the Histogram diff copied from gix-imara-diff into `crates/diffyman-algo/src/line_diff/` (`histogram.rs` and the `histogram/` folder). Those files are Apache-2.0 only, so the `diffyman-algo` crate is licensed `(MIT OR Apache-2.0) AND Apache-2.0`. See [their NOTICE](crates/diffyman-algo/src/line_diff/histogram/NOTICE).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

---

Made by [Sankara Reddy Telukutla](https://reddy.sh) · [reddy.sh](https://reddy.sh)
