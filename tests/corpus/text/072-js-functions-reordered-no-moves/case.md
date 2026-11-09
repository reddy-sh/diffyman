# 072-js-functions-reordered-no-moves

Three JavaScript functions reordered (the last one moved to the top), with `detectMoves: false`. The line diff keeps the two functions that stay in order and reports the moved one as a deletion at the end and an insertion at the top. The seven equal lines between them are more than 2 x 3, so a correct result has two hunks: h1 with `insert` R1-4 (the function and a blank line) and equal L1-3/R5-7, h2 with equal L5-7/R9-11 and `delete` L8-11 (the blank line and the function). `moves: []`, `linesAdded: 4`, `linesRemoved: 4`, `linesUnchanged: 7`.
