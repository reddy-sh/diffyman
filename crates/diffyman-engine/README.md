# diffyman

A headless "diff any file" engine. Compare two inputs (text, binary files, folders or zip and
tar archives) and get one versioned, canonical JSON result: the
[Diff Document](https://github.com/reddy-sh/diffyman/blob/main/docs/spec/diff-document.md).
The same bytes come from the CLI, the Python, Node.js, Java, WASM and C bindings and the REST
service, once the top-level `provenance` key is removed.

```toml
[dependencies]
diffyman = "0.1.0"
```

```rust
let json = diffyman::diff(b"a\nb\n", b"a\nc\n", r#"{"context": 1}"#).unwrap();
assert!(json.contains(r#""status":"different""#));
```

- `diff` and `diff_inputs` return the canonical JSON text (RFC 8785).
- `diff_document` returns the same document as Rust values (`diffyman::model`).
- `diff_paths` compares two files, two folders, or a folder and an archive on disk.

Invalid options and unreadable paths are an `Error`; problems with the inputs themselves are a
document with `status: "error"` and the reason in `warnings`.

## Features

| Feature | Default | What it adds |
|---|---|---|
| `archives` | yes (through `fs`) | zip, tar, tar.gz and tar.zst compared as trees, from bytes |
| `fs` | yes | folders on disk (`diff_paths`); implies `archives` |

Build with `default-features = false` for `wasm32-unknown-unknown`. A build without `archives`
gives the same document for every other pair and refuses the pairs that need it with
`Error::Unsupported`.

## Links

- [Documentation](https://reddy-sh.github.io/diffyman)
- [Diff Document specification](https://github.com/reddy-sh/diffyman/blob/main/docs/spec/diff-document.md)
- [Command-line tool](https://crates.io/crates/diffyman-cli) (`cargo install diffyman-cli`)
- [Repository](https://github.com/reddy-sh/diffyman)

## License

MIT OR Apache-2.0, at your option. The `diffyman-algo` dependency also holds Apache-2.0-only
files (see its NOTICE).
