# 004-simple-line-edit

Line 5 of a ten-line file changes; nothing else does. The edit is unambiguous, so every algorithm gives the same script; this case selects `myers` to exercise that option. A correct result has one hunk `left {2, 7}` / `right {2, 7}` made of equal L2-4, `replace` L5/R5 and equal L6-8 (three lines of context each way), with `linesAdded: 1`, `linesRemoved: 1`, `linesUnchanged: 9` and `similarity: 0.9`.
