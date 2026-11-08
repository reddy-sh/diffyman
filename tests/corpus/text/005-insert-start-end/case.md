# 005-insert-start-end

Two lines are inserted before line 1 and one line is appended after the last line of an eight-line file. The eight shared lines exceed 2 x context (6), so a correct result has two hunks: the first holds an `insert` with `leftRange {start: 1, count: 0}` and `rightRange {start: 1, count: 2}` followed by three equal lines; the second holds three equal lines and an `insert` with `leftRange {start: 9, count: 0}` (N + 1 for an append) and `rightRange {start: 11, count: 1}`.
