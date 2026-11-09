# Quickstart: Python

The `diffyman` package compares bytes, text, files, folders and archives with the same Rust
engine as the CLI (through PyO3) and returns the [Diff Document](../spec/diff-document.md):
as canonical JSON text, byte for byte what `diffyman diff --json` prints apart from
`provenance`, or parsed into a `dict`.

## What you need

- Python 3.11 or newer (one abi3 wheel covers every later version).
- git, and the Rust toolchain pinned in [`rust-toolchain.toml`](../../rust-toolchain.toml)
  ([rustup](https://rustup.rs) installs it on the first build).

## Build and install from source

Nothing is on PyPI yet. From a clone of the repository, build a wheel with maturin:

<!-- check: build -->
```sh
python3 -m venv .venv
.venv/bin/pip install maturin==1.16.0
.venv/bin/maturin build --release -m bindings/python/Cargo.toml --out dist
```

Then install the wheel into your project's environment:

<!-- check: skip installs into your own environment -->
```sh
pip install path/to/diffyman/dist/diffyman-*.whl
```

## Diff two files

Two small files to compare:

<!-- check: run -->
```sh
printf 'apple\nbanana\ncherry\n' > old.txt
printf 'apple\nblueberry\ncherry\ndate\n' > new.txt
```

The document has line ranges, not text, so the example cuts each input into lines (Python's
text mode turns CRLF and CR into LF, and the engine ends a line at each of the three) and
prints the lines of every change op. Save it as `example.py`:

<!-- check: file example.py -->
```python
import sys
from pathlib import Path

import diffyman

left, right = Path(sys.argv[1]), Path(sys.argv[2])
doc = diffyman.diff_document(left, right)  # a dict; diffyman.diff() gives the JSON text
print(doc["status"])  # identical, different, partial or error

old = left.read_text(encoding="utf-8-sig").split("\n")
new = right.read_text(encoding="utf-8-sig").split("\n")
for hunk in doc["hunks"]:
    for op in hunk["ops"]:
        if op["op"] in ("insert", "delete", "replace"):
            lr, rr = op["leftRange"], op["rightRange"]
            for line in old[lr["start"] - 1 : lr["start"] - 1 + lr["count"]]:
                print("-", line)
            for line in new[rr["start"] - 1 : rr["start"] - 1 + rr["count"]]:
                print("+", line)
```

<!-- check: run -->
```sh
python example.py old.txt new.txt
```

<!-- check: output -->
```text
different
- banana
+ blueberry
+ date
```

`diff` and `diff_document` take `bytes`, `str` (the text itself, never a file name) or
`os.PathLike` inputs; a path's file name becomes `left.name` or `right.name`. Two folders, or
a folder and an archive, give a document of kind `tree` with one entry per path
([spec §6.8](../spec/diff-document.md#68-tree-tree-since-030)); two binary files give kind
`binary` with changed byte ranges ([§6.7](../spec/diff-document.md#67-byte-ranges-bytes-since-030)).
The `TypedDict` classes in `diffyman.types` (`DiffDocument`, `Hunk`, `Op`, ...) type the
parsed document for mypy and pyright.

## Options and inline changes

Options are a `dict` (or JSON text) in the shape of
[spec §9](../spec/diff-document.md#9-options). With a `granularity` other than `line`,
`replace` ops carry inline spans; `positionEncoding: "codepoint"` makes their offsets index a
Python `str` directly:

<!-- check: file words.py -->
```python
import diffyman

left, right = "the quick brown fox\n", "the slow brown fox\n"
options = {"granularity": "word", "positionEncoding": "codepoint"}
doc = diffyman.diff_document(left, right, options)
lines = {"left": left.split("\n"), "right": right.split("\n")}
for hunk in doc["hunks"]:
    for op in hunk["ops"]:
        for span in op["inline"]:
            text = lines[span["side"]][span["line"] - 1][span["start"] : span["end"]]
            print(span["op"], repr(text))
```

<!-- check: run -->
```sh
python words.py
```

<!-- check: output -->
```text
delete 'quick'
insert 'slow'
```

The binding has no renderer. For an HTML report or a patch, run the CLI
(`diffyman diff old.txt new.txt -f html -o report.html`, `-f unified`; see the
[CLI quickstart](cli.md)), or render the document yourself as above.

## Errors

Invalid options (an unknown key, a wrong type, an unknown value) raise
`diffyman.DiffymanError`, a `ValueError`, with the engine's message and no document. A path
that cannot be read raises `OSError`. Problems with the inputs themselves are a document
with `status: "error"` and the reason in `warnings`
([spec §10](../spec/diff-document.md#10-warnings-and-errors)):

<!-- check: file errors.py -->
```python
from pathlib import Path

import diffyman

try:
    diffyman.diff_document("a\n", "b\n", {"granularity": "sentence"})
except diffyman.DiffymanError as err:
    print("DiffymanError:", err)

try:
    diffyman.diff_document(Path("missing.txt"), Path("new.txt"))
except OSError as err:
    print("OSError:", err)

doc = diffyman.diff_document("a\n", "b\n", {"ignore": {"regex": ["("]}})
for warning in doc["warnings"]:
    print(doc["status"], warning["severity"], warning["code"], warning["message"])
```

<!-- check: run -->
```sh
python errors.py
```

<!-- check: output -->
```text
DiffymanError: invalid options: unknown value for granularity
OSError: cannot read missing.txt: No such file or directory (os error 2)
error error invalid-regex An ignore.regex pattern is not a valid regular expression.
```

Act on `warning["code"]`, never on the message text.

## Next

- [Python binding README](../../bindings/python/README.md): the full API, threads, types and
  development.
- [Diff Document specification](../spec/diff-document.md): every field, option and warning.
- [CLI quickstart](cli.md) for HTML and unified output.
