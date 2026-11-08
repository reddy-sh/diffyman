# 009-crlf-vs-lf

The same five lines end in LF on the left and CRLF on the right. Under `ignore.whitespace: "none"` the terminator kind is part of the key, so every line differs: a correct result has one hunk with a single `replace` op covering L1-5 and R1-5, `linesUnchanged: 0` and `similarity: 0`. The left side reports `lineTerminator: "lf"`, the right side `"crlf"`, and both have `finalNewline: true`, the same `lineCount: 5` and no warnings.
