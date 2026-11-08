# 025-lone-cr-vs-lf

The left file ends every line with a lone CR, the right file with LF, and line 4 has a real change. Runs `ignore.whitespace: "trailing"`, where the terminator is not part of the key. CR is a terminator, not content (and not whitespace under spec 7.2), so lines 1-3 and 5-6 compare equal. A correct result has one hunk: equal L1-3, `replace` L4/R4, equal L5-6; `linesUnchanged: 5`. The left side reports `lineTerminator: "cr"`, the right `"lf"`, both `lineCount: 6`, and there is no `mixed-line-endings` warning.
