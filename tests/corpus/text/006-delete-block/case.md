# 006-delete-block

Five consecutive lines (L6-L10) are deleted from a fifteen-line checklist; every line is unique, so patience gives the same script as the other algorithms. With `context: 2` a correct result has one hunk `left {4, 9}` / `right {4, 4}`: equal L4-5, a `delete` with `leftRange {start: 6, count: 5}` and `rightRange {start: 6, count: 0}`, then equal L11-12. Stats show `linesRemoved: 5`, `linesUnchanged: 10`.
