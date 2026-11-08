# 003-one-empty

The left file is empty and the right file has three LF lines. A correct result has one hunk holding one `insert` op with `leftRange {start: 1, count: 0}` and `rightRange {start: 1, count: 3}`; the hunk ranges are the same. Stats show `linesAdded: 3`, `linesUnchanged: 0` and `similarity: 0`. The empty side reports `lineTerminator: "none"` and no `no-newline-at-eof` warning, because it has no last line.
