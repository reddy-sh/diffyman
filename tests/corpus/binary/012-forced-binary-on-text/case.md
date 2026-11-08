# 012-forced-binary-on-text

Two UTF-8 text files with `adapter: "binary"`. A correct result is `kind: "binary"` without `binary-fallback` (the adapter was asked for) and without text warnings, both sides `encoding: "utf-8"` with null line fields, and two ranges: `beta` -> `BETA` (4 bytes at offset 6) and the CR inserted at left offset 22, before the last LF.
