"""Type stubs for the native extension (bindings/python/src/lib.rs)."""

import os

ENGINE_VERSION: str
SCHEMA_VERSION: str
MAX_STREAM_BYTES: int

class DiffymanError(ValueError):
    """Raised when the options are invalid (unknown key, wrong type or unknown value)."""

def diff(
    left: bytes,
    right: bytes,
    options_json: str = "",
    left_name: str | None = None,
    right_name: str | None = None,
) -> str: ...

def diff_paths(
    left: str | os.PathLike[str],
    right: str | os.PathLike[str],
    options_json: str = "",
    limit: int | None = None,
) -> str: ...

def read_path(path: str | os.PathLike[str], limit: int | None = None) -> bytes: ...
