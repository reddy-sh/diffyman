# 053-json-reindent-whitespace-change

A JSON file (as text: `mediaType: "application/json"`) re-indented from two to four spaces, with an aligned value on L6 and a new version on L3, under `ignore.whitespace: "change"`. Leading indentation of any width is one run of whitespace, so it collapses to one space on both sides and the re-indented lines compare equal. A correct result has one hunk: equal L1-2, `replace` L3/R3, equal L4-6; `linesUnchanged: 8`, `similarity: 0.8889`.
