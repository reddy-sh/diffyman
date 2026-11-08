# 016-invisible-chars

Runs `ignore.whitespace: "all"`, which removes only U+0009, U+000B, U+000C and U+0020. Line 3 differs only in runs of spaces and compares equal. Lines 1, 2 and 4 gain invisible characters that are not whitespace and must still differ: U+FEFF inside a line (not at file start, so it is content, not a BOM), U+200B ZERO WIDTH SPACE, and U+202E RIGHT-TO-LEFT OVERRIDE disguising a file extension. A correct result with `context: 5` has one hunk covering the whole file: `replace` L1-2, equal L3, `replace` L4, equal L5; no `bom-stripped` warning.
