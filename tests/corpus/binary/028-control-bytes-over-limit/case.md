# 028-control-bytes-over-limit

Valid UTF-8 without NUL bytes, named `.txt`: 40 records of 12 bytes, each with three control bytes (U+0001-U+0003), so 25 % of the sniff window (the whole 480-byte input) is control bytes, more than the 10 % a text input may have (spec 16.1). Both sides are binary, so there is no `binary-fallback`, and the `txt` extension applies to text only: a correct result has `mediaType: "application/octet-stream"` and `encoding: null` on both sides, and one `replace` range of 5 bytes at offset 245 (`value` -> `VALUE` in record 21).
