# 023-mixed-line-endings

Both files mix LF, a lone CR (after line 3) and CRLF in one file. The right side changes the URL on line 2 and ends line 5 with LF instead of CRLF. Each terminator ends a line, so both sides have `lineCount: 6`, `lineTerminator: "mixed"` and a `mixed-line-endings` warning. Under `ignore.whitespace: "none"` the terminator kind is part of the key, so a correct result has one hunk covering L1-6: equal L1, `replace` L2, equal L3-4, `replace` L5 (same content, CRLF versus LF), equal L6; `linesUnchanged: 4` and `similarity: 0.6667`.
