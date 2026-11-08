# 008-no-final-newline

Both sides hold the same six lines, but the right file has no LF after its last line. Under `ignore.whitespace: "none"` the terminator kind is part of the comparison key, so a correct result is `different` with one hunk: equal L3-5 and a `replace` of L6/R6. The right side has `finalNewline: false`, `lineTerminator: "lf"` (every terminated line uses LF) and a `no-newline-at-eof` warning; the left side has neither.
