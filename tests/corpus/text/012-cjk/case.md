# 012-cjk

Chinese, Japanese, Korean and full-width Latin lines. Line 2 changes two Han characters and a Japanese line is inserted after line 3. With `context: 1` the single equal line between the two changes is within 2 x context, so a correct result has one hunk: equal L1, `replace` L2/R2, equal L3, `insert` before L4 (`rightRange {start: 4, count: 1}`), equal L4/R5. `positionEncoding` is `utf-16`; with line granularity it only shows up in the echoed options.
