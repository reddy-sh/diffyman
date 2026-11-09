# 046-ts-patience-new-method

A TypeScript class gains a `remove` method between `add` and `total`, run with `algorithm: "patience"`. Patience anchors on lines that occur once on each side; the closing `  }` and the blank lines occur several times, so the inserted method could be described at more than one position. A correct result has one hunk: equal L5-7, a single `insert` of R8-11 (`remove(...) {`, its body, `  }` and a blank line) and equal L8-10/R12-14. Slider compaction puts the insertion at the method header, as git's patience diff does. Stats: `linesAdded: 4`, `linesUnchanged: 15`.
