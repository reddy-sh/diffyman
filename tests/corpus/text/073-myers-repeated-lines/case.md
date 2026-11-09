# 073-myers-repeated-lines

Lines that repeat three times, run with `algorithm: "myers"`: a `check` line is inserted into the second group and an extra `end` is appended. Many alignments are minimal; slider compaction puts each insertion in its canonical place. A correct result is a minimal edit script with two one-line inserts (`check` at R6 and `end` at R11), `linesAdded: 2`, `linesRemoved: 0`, `linesUnchanged: 9`, and the same hunks as `git diff --diff-algorithm=myers`.
