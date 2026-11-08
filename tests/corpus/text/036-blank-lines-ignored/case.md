# 036-blank-lines-ignored

The inputs of case 026, run with `ignore.blankLines: true` (spec 7.3 step 3). Every change op holds only blank lines, so each becomes an `ignored` op, which is not a change op and never starts a hunk. A correct result is `identical` with `hunks: []`, `changeCount: 0`, `hunkCount: 0`, `linesAdded: 0`, `linesRemoved: 0` and `linesUnchanged: 8`. The three ignored lines (one left, two right) leave the similarity denominator (spec 6.3), so `similarity` is `1` = 2 x 8 / (8 + 8).
