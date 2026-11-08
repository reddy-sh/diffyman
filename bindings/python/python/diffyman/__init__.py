"""diffyman: compare two inputs and get one canonical JSON Diff Document.

The result follows ``docs/spec/diff-document.md`` (schema ``0.2.0``). The JSON
returned by :func:`diff` is byte-identical to ``diffyman diff --json`` and to
every other binding once the top-level ``provenance`` key is removed.

>>> import diffyman
>>> doc = diffyman.diff_document(b"a\\nb\\n", b"a\\nc\\n", {"context": 1})
>>> doc["status"]
'different'

Inputs may be:

- ``bytes``, ``bytearray`` or ``memoryview``: used as is.
- ``str``: the text itself, encoded as UTF-8. A ``str`` is never treated as a
  file name; wrap paths in :class:`pathlib.Path`.
- ``os.PathLike`` (for example :class:`pathlib.Path`): the file is read as
  bytes, and its final path component becomes the input name unless
  ``left_name``/``right_name`` is given.

Options may be a mapping (serialized to JSON), a JSON string, or ``None`` for
the defaults. Invalid options raise :class:`DiffymanError` (a ``ValueError``)
and produce no document.
"""

from __future__ import annotations

import json
import os
from collections.abc import Mapping
from typing import Any, Union

from ._native import ENGINE_VERSION, SCHEMA_VERSION, DiffymanError
from ._native import diff as _native_diff

__all__ = [
    "ENGINE_VERSION",
    "SCHEMA_VERSION",
    "DiffymanError",
    "Options",
    "Source",
    "__version__",
    "diff",
    "diff_document",
]

__version__ = ENGINE_VERSION

Source = Union[bytes, bytearray, memoryview, str, os.PathLike[str], os.PathLike[bytes]]
"""What :func:`diff` accepts as an input."""

Options = Union[Mapping[str, Any], str, None]
"""An options object (diff-document.md section 9), as a mapping or JSON text."""


def _load(source: Source, name: str | None) -> tuple[bytes, str | None]:
    if isinstance(source, bytes):
        return source, name
    if isinstance(source, (bytearray, memoryview)):
        return bytes(source), name
    if isinstance(source, str):
        return source.encode("utf-8"), name
    if isinstance(source, os.PathLike):
        path = os.fsdecode(os.fspath(source))
        with open(path, "rb") as handle:
            data = handle.read()
        return data, name if name is not None else os.path.basename(path)
    raise TypeError(
        "diffyman inputs must be bytes, bytearray, memoryview, str or os.PathLike, "
        f"not {type(source).__name__}"
    )


def _options_json(options: Options) -> str:
    if options is None:
        return ""
    if isinstance(options, str):
        return options
    if isinstance(options, Mapping):
        return json.dumps(dict(options), ensure_ascii=False, allow_nan=False)
    raise TypeError(
        "diffyman options must be a mapping, a JSON string or None, "
        f"not {type(options).__name__}"
    )


def diff(
    left: Source,
    right: Source,
    options: Options = None,
    *,
    left_name: str | None = None,
    right_name: str | None = None,
) -> str:
    """Diff two inputs and return the canonical (RFC 8785) Diff Document as text.

    ``left_name`` and ``right_name`` set ``left.name`` and ``right.name``, which
    are part of the document ``id``. Only the final path component is kept.
    """
    left_bytes, left_name = _load(left, left_name)
    right_bytes, right_name = _load(right, right_name)
    return _native_diff(left_bytes, right_bytes, _options_json(options), left_name, right_name)


def diff_document(
    left: Source,
    right: Source,
    options: Options = None,
    *,
    left_name: str | None = None,
    right_name: str | None = None,
) -> dict[str, Any]:
    """Like :func:`diff`, but return the parsed document as a ``dict``."""
    result: dict[str, Any] = json.loads(
        diff(left, right, options, left_name=left_name, right_name=right_name)
    )
    return result
