# 006-nul-bytes

Valid UTF-8 with NUL bytes, as in a NUL-separated record file, named `.txt`. A NUL in the first 8000 bytes makes an input binary (spec 16.1), so a correct result is `kind: "binary"` without `binary-fallback` (neither side is text), `mediaType: "application/octet-stream"` on both sides (the `txt` extension applies to text only) and one `replace` range of length 1 at offset 17.
