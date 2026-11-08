# 024-lone-cr

Both files use only a lone CR as the terminator (classic Mac OS style), including after the empty line 5 and the last line. A lone CR ends a line (spec 7.2), so both sides have `lineCount: 6`, `lineTerminator: "cr"`, `finalNewline: true` and no warnings. Line 3 changes. A correct result has one hunk: equal L1-2, `replace` L3/R3, equal L4-6; `linesUnchanged: 5` and `similarity: 0.8333`. A splitter that treats CR as content would see one line per side instead.
