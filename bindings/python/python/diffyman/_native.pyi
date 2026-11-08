"""Type stubs for the native extension (bindings/python/src/lib.rs)."""

ENGINE_VERSION: str
SCHEMA_VERSION: str

class DiffymanError(ValueError):
    """Raised when the options are invalid (unknown key, wrong type or unknown value)."""

def diff(
    left: bytes,
    right: bytes,
    options_json: str = "",
    left_name: str | None = None,
    right_name: str | None = None,
) -> str: ...
