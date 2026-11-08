# 010-utf8-bom

The right file is the left file with a UTF-8 byte order mark (EF BB BF) in front. The BOM is removed before splitting into lines, so a correct result is `identical` with no hunks. The right side reports `bom: true` and a `size` three bytes larger than the left, the two `sha256` values differ, and `warnings` holds exactly one `bom-stripped` entry with `side: "right"`.
