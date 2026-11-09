# diffyman for Python

Python binding for [diffyman](https://github.com/reddy-sh/diffyman), a headless
"diff any file" engine: text, binary files, folders and archives, compared by
one Rust core through PyO3. One abi3 wheel per platform covers CPython 3.11
and newer.

## Example

```python
import pathlib
import diffyman

doc = diffyman.diff_document(b"a\nb\nc\n", b"a\nB\nc\n", {"granularity": "word"})
print(doc["status"], doc["stats"]["similarity"] if doc["stats"] else None)
for hunk in doc["hunks"]:
    for op in hunk["ops"]:
        print(op["op"], op["leftRange"], op["rightRange"])

text = diffyman.diff(pathlib.Path("old.txt"), pathlib.Path("new.txt"))  # canonical JSON
tree = diffyman.diff_document(pathlib.Path("v1"), pathlib.Path("v2"))   # two folders
```

## API

- `diff(left, right, options=None, *, left_name=None, right_name=None, max_input_size=None) -> str`
  returns the canonical (RFC 8785) Diff Document. It is byte-identical to
  `diffyman diff --json` and to every other binding once the top-level
  `provenance` key is removed.
- `diff_document(...) -> DiffDocument` returns the same document parsed: a
  plain `dict`, typed with the `TypedDict` classes in `diffyman.types`.
- `document_schema() -> dict` returns the document's JSON Schema, the same
  as `diffyman schema`.
- Inputs are `bytes`-like (used as is), `str` (the text itself, UTF-8, never a
  file name) or `os.PathLike` (read from disk; the file name becomes the input
  name). Two paths of which one is a folder give a tree document.
- Options are a mapping, a JSON string or `None` (the options are listed in
  `docs/spec/diff-document.md`, section 9). Invalid options raise
  `diffyman.DiffymanError` (a `ValueError`) with the engine's message; an
  unreadable path raises `OSError`.
- The GIL is released while the diff runs, so threads can diff in parallel.

### Types

`diffyman.types` is generated from the JSON Schema by
[`tools/gen_types.py`](tools/gen_types.py) (standard library only) and never
edited by hand. Every object in the document is a `TypedDict` (`DiffDocument`,
`Hunk`, `Op`, `Span`, `Move`, `TreeEntry`, ...) and every enum a `Literal`
alias (`Status`, `OpKind`, `WarningCode`, ...). The schema's `Warning` is
`DiffWarning` here, so it does not shadow the built-in. Nullable fields are
`X | None`, so a type checker asks you to handle a `null` `stats` or `tree`.
The package ships `py.typed` and passes `mypy --strict`.

## Install from source

You need a Rust toolchain (see `rust-toolchain.toml`) and Python 3.11 or newer.
From the repository root:

```sh
python -m venv .venv
.venv/bin/pip install maturin==1.16.0
.venv/bin/maturin build --release --sdist -m bindings/python/Cargo.toml --out dist
.venv/bin/pip install dist/diffyman-*.whl
```

`maturin build` writes an sdist and an abi3 wheel for the current platform to
`dist/`; the wheel can also be built from the sdist alone
(`pip install dist/diffyman-*.tar.gz`, which needs the Rust toolchain).

## Development

From the repository root (`scripts/with-rustup` puts the pinned toolchain
first on `PATH`):

```sh
uv venv --python 3.12 bindings/python/.venv
uv pip install --python bindings/python/.venv/bin/python maturin==1.16.0 pytest==9.2.0 mypy==2.4.0
VIRTUAL_ENV=bindings/python/.venv scripts/with-rustup bindings/python/.venv/bin/maturin develop --uv --release -m bindings/python/Cargo.toml
bindings/python/.venv/bin/python -m pytest bindings/python/tests
bindings/python/.venv/bin/mypy --config-file bindings/python/pyproject.toml
```

The same pins are in `pyproject.toml` as dependency groups (`test`,
`typecheck`, `dev`), which are not part of the published metadata.
`uv.lock` pins their transitive dependencies too (`uv lock --check` from this
folder verifies it, `uv sync --group dev` installs from it).

After a schema change, regenerate the types and commit the result; the tests
fail while `types.py` is out of date:

```sh
bindings/python/.venv/bin/python bindings/python/tools/gen_types.py          # write
bindings/python/.venv/bin/python bindings/python/tools/gen_types.py --check  # verify
```

The generator reads the schema from the installed extension by default, or
from a file with `--schema` (`diffyman schema > schema.json`). With
`DIFFYMAN_CLI` set to a built CLI, the tests also check that the extension's
schema is the text `diffyman schema` prints.

## License

MIT OR Apache-2.0, at your option. `LICENSE-MIT` and `LICENSE-APACHE` in this
folder (copies of the files at the repository root, listed in `license-files`)
ship in the sdist and the wheel.
