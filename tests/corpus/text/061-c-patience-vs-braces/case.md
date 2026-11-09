# 061-c-patience-vs-braces

C functions in Allman style, where `{`, `}` and blank lines repeat after every header; a function `mul` is inserted between `add` and `sub` and `main` calls it. Run with `algorithm: "patience"`. A correct result has two hunks, because seven equal lines (L8-14) separate the changes: h1 with an `insert` of R8-12 (the header of `mul` through the blank line after it, placed at the header by slider compaction), h2 with a `replace` of L15/R20, as git's patience diff gives. Stats: `linesAdded: 6`, `linesRemoved: 1`, `linesUnchanged: 16`.
