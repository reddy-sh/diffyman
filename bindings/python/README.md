# diffyman for Python

Python binding for [diffyman](https://github.com/reddy-sh/diffyman), a headless
"diff any file" engine. One Rust core, called through PyO3 (abi3 wheels for
CPython 3.11 and newer).

```python
import pathlib
import diffyman

json_text = diffyman.diff(pathlib.Path("old.txt"), pathlib.Path("new.txt"), {"context": 1})
doc = diffyman.diff_document(b"a\nb\n", b"a\nc\n")
print(doc["status"], doc["stats"]["linesAdded"])
```

- `diff(left, right, options=None, *, left_name=None, right_name=None) -> str`
  returns the canonical (RFC 8785) Diff Document. It is byte-identical to
  `diffyman diff --json` once the top-level `provenance` key is removed.
- `diff_document(...) -> dict` returns the same document parsed.
- Inputs are `bytes`-like (used as is), `str` (the text itself, UTF-8) or
  `os.PathLike` (read from disk; the file name becomes the input name).
- Options are a mapping, a JSON string or `None`. Invalid options raise
  `diffyman.DiffymanError` (a `ValueError`).
- The GIL is released while the diff runs.

The document format is specified in `docs/spec/diff-document.md`.

## Development

From the repository root:

```sh
uv venv --python 3.12 bindings/python/.venv
uv pip install --python bindings/python/.venv/bin/python pytest==9.2.0
scripts/with-rustup maturin develop --release -m bindings/python/Cargo.toml
bindings/python/.venv/bin/python -m pytest bindings/python/tests
```

`maturin develop` installs into the active virtual environment, so either
activate `.venv` first or set `VIRTUAL_ENV` to it.

## License

MIT OR Apache-2.0.
