# 042-pure-insert-repeated-lines

The right file inserts 600 more copies of a separator line that the left file already has twice. Once the common prefix and suffix are trimmed the left side is empty, so the diff is a pure insertion: no search runs and the result is exact, though more than 500 changed lines have a key that occurs on the other side (spec 10). A correct result has no `approximate` warning: one hunk with an `insert` op of 600 lines, `linesAdded: 600`, and `linesUnchanged: 5`.
