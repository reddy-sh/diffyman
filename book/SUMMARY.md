# Summary

[Introduction](README.md)

# Quickstarts

- [Command line](docs/quickstart/cli.md)
- [Python](docs/quickstart/python.md)
- [Node.js](docs/quickstart/node.md)
- [Java](docs/quickstart/java.md)
- [Browser and React](docs/quickstart/browser-react.md)
- [C](docs/quickstart/c.md)
- [REST API](docs/quickstart/rest.md)

# Reference

- [Command-line reference](docs/cli.md)
- [REST API reference](docs/api/README.md)
  - [OpenAPI reference](book/openapi.md)
  - [Running diffyman-server](docs/server.md)
  - [Operating the local service](docs/operations.md)
- [Specifications](docs/spec/README.md)
  - [Diff Document](docs/spec/diff-document.md)
  - [Text granularity and moved blocks](docs/spec/text-granularity-and-moves.md)
- [Benchmarks](docs/benchmarks.md)

# Project

- [Architecture decisions](docs/adr/README.md)
  - [0001 Rust core](docs/adr/0001-rust-core.md)
  - [0002 Diff Document contract](docs/adr/0002-diff-document-contract.md)
  - [0003 PostgreSQL over SQLite](docs/adr/0003-postgres-over-sqlite.md)
  - [0004 Object storage](docs/adr/0004-object-storage.md)
  - [0005 License](docs/adr/0005-license.md)
  - [0006 Line diff library](docs/adr/0006-line-diff-library.md)
  - [0007 Binding strategy](docs/adr/0007-binding-strategy.md)
  - [0008 WASM target](docs/adr/0008-wasm-target.md)
  - [0009 Type detection and binary diff](docs/adr/0009-type-detection-and-binary-diff.md)
  - [0010 Folders and archives](docs/adr/0010-folders-and-archives.md)
  - [0011 Inline spans in edited moves](docs/adr/0011-inline-spans-in-edited-moves.md)
  - [0012 Local service architecture](docs/adr/0012-local-service-architecture.md)
- [Status](docs/STATUS.md)
  - [Spike A: line diff](docs/spikes/A-line-diff.md)
  - [Spike B: binding round trip](docs/spikes/B-binding-round-trip.md)
  - [Spike C: WASM build](docs/spikes/C-wasm-build.md)
- [Contributing](CONTRIBUTING.md)
- [Releasing](docs/release.md)
