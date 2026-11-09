# 059-large-csv-myers

A generated CSV export of 1701 lines (about 48 KB), run with `algorithm: "myers"`: the amount on row L52 changes, ten new rows are inserted after L701 and rows L1202-1206 are deleted. Every row has a unique id. A correct result has three hunks: a one-line `replace` at L52, an `insert` of ten lines after L701 and a `delete` of five lines at L1202; `linesAdded: 11`, `linesRemoved: 6`, `linesUnchanged: 1695`, and `mediaType: "text/csv"` on both sides.
