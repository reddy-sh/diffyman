# 063-context-eight-joins-hunks

Two one-line edits fifteen lines apart (L5 and L21) in a 30-line file, with `context: 8`. Fifteen equal lines between them are at most 2 x 8, so they stay in one hunk: a correct result has one hunk covering L1-29 (four equal lines before L5, the fifteen between, eight after L21), `hunkCount: 1`, `changeCount: 2` and `linesUnchanged: 28`.
