"""diffyman: compare two inputs and get one canonical JSON Diff Document.

The result follows ``docs/spec/diff-document.md`` (schema ``0.4.0``). The JSON
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
  ``left_name``/``right_name`` is given. Two paths of which one is a folder
  are compared as folders and archives (a tree document).

Paths are read with the engine's bounded reader, like the CLI: a regular
file whole, a pipe or device up to :data:`MAX_STREAM_BYTES` (4 GiB).
``max_input_size`` sets one byte limit for every path input; an input over
the limit raises :class:`OSError` (``limit-exceeded: ...``).

Options may be a mapping (serialized to JSON), a JSON string, or ``None`` for
the defaults. Invalid options raise :class:`DiffymanError` (a ``ValueError``)
and produce no document.

:func:`diff_document` returns a plain ``dict`` typed as
:class:`diffyman.types.DiffDocument`; the ``TypedDict`` classes in
:mod:`diffyman.types` are generated from the JSON Schema
(:func:`document_schema`, the same text as ``diffyman schema``).
"""

from __future__ import annotations

import json
import os
from collections.abc import Mapping
from typing import Any, Union

from ._native import ENGINE_VERSION, MAX_STREAM_BYTES, SCHEMA_VERSION, DiffymanError
from ._native import diff as _native_diff
from ._native import diff_paths as _native_diff_paths
from ._native import document_schema as _native_document_schema
from ._native import read_path as _native_read_path
from .types import DiffDocument

__all__ = [
    "ENGINE_VERSION",
    "MAX_STREAM_BYTES",
    "SCHEMA_VERSION",
    "DiffDocument",
    "DiffymanError",
    "Options",
    "Source",
    "__version__",
    "diff",
    "diff_document",
    "document_schema",
]

__version__ = ENGINE_VERSION

Source = Union[bytes, bytearray, memoryview, str, os.PathLike[str], os.PathLike[bytes]]
"""What :func:`diff` accepts as an input."""

Options = Union[Mapping[str, Any], str, None]
"""An options object (diff-document.md section 9), as a mapping or JSON text."""


def _check_limit(max_input_size: int | None) -> int | None:
    if max_input_size is None:
        return None
    if isinstance(max_input_size, bool) or not isinstance(max_input_size, int):
        raise TypeError(
            f"max_input_size must be an int or None, not {type(max_input_size).__name__}"
        )
    if max_input_size < 0:
        raise ValueError("max_input_size must not be negative")
    return max_input_size


def _load(source: Source, name: str | None, limit: int | None) -> tuple[bytes, str | None]:
    if isinstance(source, bytes):
        return source, name
    if isinstance(source, (bytearray, memoryview)):
        return bytes(source), name
    if isinstance(source, str):
        return source.encode("utf-8"), name
    if isinstance(source, os.PathLike):
        path = os.fsdecode(os.fspath(source))
        data = _native_read_path(path, limit)
        return data, name if name is not None else os.path.basename(path)
    raise TypeError(
        "diffyman inputs must be bytes, bytearray, memoryview, str or os.PathLike, "
        f"not {type(source).__name__}"
    )


def _is_folder_pair(left: Source, right: Source) -> bool:
    """Both inputs are paths, and at least one is a folder."""
    if not (isinstance(left, os.PathLike) and isinstance(right, os.PathLike)):
        return False
    return os.path.isdir(os.fspath(left)) or os.path.isdir(os.fspath(right))


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
    max_input_size: int | None = None,
) -> str:
    """Diff two inputs and return the canonical (RFC 8785) Diff Document as text.

    ``left_name`` and ``right_name`` set ``left.name`` and ``right.name``, which
    are part of the document ``id``. Only the final path component is kept.

    When both inputs are paths and one of them is a folder, the result is a
    tree document (folders and archives, diff-document.md section 19); the
    names are then the folders' own names and ``left_name``/``right_name``
    must not be given.

    ``max_input_size`` is a byte limit for every path input (and the archive
    side of a folder diff), like the CLI's ``--max-input-size``. ``None`` keeps
    the default: regular files whole, pipes and devices up to
    :data:`MAX_STREAM_BYTES`.
    """
    limit = _check_limit(max_input_size)
    if _is_folder_pair(left, right):
        if left_name is not None or right_name is not None:
            raise TypeError("left_name and right_name cannot be used with folders")
        return _native_diff_paths(
            os.fspath(left), os.fspath(right), _options_json(options), limit  # type: ignore[arg-type]
        )
    left_bytes, left_name = _load(left, left_name, limit)
    right_bytes, right_name = _load(right, right_name, limit)
    return _native_diff(left_bytes, right_bytes, _options_json(options), left_name, right_name)


def diff_document(
    left: Source,
    right: Source,
    options: Options = None,
    *,
    left_name: str | None = None,
    right_name: str | None = None,
    max_input_size: int | None = None,
) -> DiffDocument:
    """Like :func:`diff`, but return the parsed document as a ``dict``.

    The ``dict`` is what :func:`json.loads` gives; its type
    :class:`~diffyman.types.DiffDocument` only guides a type checker.
    """
    result: DiffDocument = json.loads(
        diff(
            left,
            right,
            options,
            left_name=left_name,
            right_name=right_name,
            max_input_size=max_input_size,
        )
    )
    return result


def document_schema() -> dict[str, Any]:
    """The JSON Schema (2020-12) of the Diff Document, as ``diffyman schema`` prints it."""
    schema: dict[str, Any] = json.loads(_native_document_schema())
    return schema
