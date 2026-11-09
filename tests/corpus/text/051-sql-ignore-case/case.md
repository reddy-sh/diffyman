# 051-sql-ignore-case

SQL keywords upper-cased, with `ignore.case: true` (Unicode simple case folding, statuses C and S). `Été` and `ÉTÉ` fold to the same text, so L1-3 and L6 compare equal. `ß` has only a full case folding (`ss`, status F), which is not applied, so `'Straße'` and `'STRASSE'` still differ. A correct result has one hunk: equal L2-4, `replace` L5/R5, equal L6; `linesUnchanged: 5`.
