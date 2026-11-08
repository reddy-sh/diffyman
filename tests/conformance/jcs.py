"""RFC 8785 JSON Canonicalization Scheme (JCS), independent of the Rust one.

The conformance harness uses this to re-serialize documents after removing
`provenance`, so the Rust canonicalizer (crates/diffyman-model/src/jcs.rs) is
checked against a second implementation on every run.

- Object keys are sorted by their UTF-16 code units.
- Strings escape only `"`, `\\` and U+0000..U+001F (`\\b \\t \\n \\f \\r`
  short forms, `\\u00xx` lowercase otherwise).
- Numbers follow ECMAScript `Number.prototype.toString`.
"""

from __future__ import annotations

import decimal
import json
import math
from typing import Any

MAX_SAFE_INTEGER = 2**53 - 1


def canonicalize(value: Any) -> bytes:
    """Return the JCS bytes of a JSON value (as produced by `json.loads`)."""
    out: list[str] = []
    _write(value, out)
    return "".join(out).encode("utf-8")


def _write(value: Any, out: list[str]) -> None:
    if value is None:
        out.append("null")
    elif value is True:
        out.append("true")
    elif value is False:
        out.append("false")
    elif isinstance(value, int):
        if abs(value) > MAX_SAFE_INTEGER:
            raise ValueError(f"integer {value} is outside the I-JSON safe range")
        out.append(str(value))
    elif isinstance(value, float):
        out.append(number(value))
    elif isinstance(value, str):
        out.append(json.dumps(value, ensure_ascii=False))
    elif isinstance(value, (list, tuple)):
        out.append("[")
        for i, item in enumerate(value):
            if i:
                out.append(",")
            _write(item, out)
        out.append("]")
    elif isinstance(value, dict):
        out.append("{")
        for i, key in enumerate(sorted(value, key=lambda k: k.encode("utf-16-be"))):
            if not isinstance(key, str):
                raise TypeError("object keys must be strings")
            if i:
                out.append(",")
            out.append(json.dumps(key, ensure_ascii=False))
            out.append(":")
            _write(value[key], out)
        out.append("}")
    else:
        raise TypeError(f"not a JSON value: {type(value).__name__}")


def number(value: float) -> str:
    """Serialize a float like ECMAScript `Number.prototype.toString`."""
    if not math.isfinite(value):
        raise ValueError("NaN and Infinity are not valid JSON")
    if value == 0:
        return "0"
    sign = "-" if value < 0 else ""
    # repr() gives the shortest digits that round-trip, as ECMAScript requires.
    digits_tuple = decimal.Decimal(repr(abs(value))).as_tuple()
    digits = "".join(map(str, digits_tuple.digits))
    exponent = int(digits_tuple.exponent)
    stripped = digits.rstrip("0")
    exponent += len(digits) - len(stripped)
    digits = stripped.lstrip("0")
    k = len(digits)
    n = exponent + k  # value = 0.digits * 10^n
    if k <= n <= 21:
        body = digits + "0" * (n - k)
    elif 0 < n <= 21:
        body = digits[:n] + "." + digits[n:]
    elif -6 < n <= 0:
        body = "0." + "0" * (-n) + digits
    else:
        e = n - 1
        mantissa = digits if k == 1 else digits[0] + "." + digits[1:]
        body = f"{mantissa}e{'+' if e >= 0 else '-'}{abs(e)}"
    return sign + body


def _self_test() -> None:
    cases = {
        0.7273: "0.7273",
        1.0: "1",
        -0.0: "0",
        1e21: "1e+21",
        1e20: "100000000000000000000",
        1.5e-7: "1.5e-7",
        0.000001: "0.000001",
        123.456: "123.456",
        5e-324: "5e-324",
        1.7976931348623157e308: "1.7976931348623157e+308",
    }
    for value, want in cases.items():
        got = number(value)
        assert got == want, f"{value!r}: {got} != {want}"
    sample = {"b": [1, 2.5, None, True], "a": "€\n\x1f\"", "": 1, "\U0001f600": 2}
    want = '{"a":"€\\n\\u001f\\"","b":[1,2.5,null,true],"\U0001f600":2,"":1}'
    assert canonicalize(sample) == want.encode("utf-8"), canonicalize(sample)


if __name__ == "__main__":
    _self_test()
    print("jcs self-test passed")
