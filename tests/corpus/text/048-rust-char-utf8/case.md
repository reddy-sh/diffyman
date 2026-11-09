# 048-rust-char-utf8

A Rust `match` with three one-character edits, run with `granularity: "char"` and `positionEncoding: "utf-8"`. A correct result has one hunk with two `replace` ops, L3/R3 and L5-6/R5-6, around equal L4. The spans are one grapheme each: `z`/`f` at byte 15 of L3/R3, `9`/`7` at byte 15 of L5/R5, and `ö`/`ő` at bytes 16-18 of L6/R6, two bytes long because both are two-byte UTF-8 characters (`ä` before them also takes two bytes).
