# 028-long-line

Line 2 is one 199,996-byte line of 28,571 numbered words; the right side changes the word in the middle. A correct result has one hunk: equal L1, `replace` L2/R2, equal L3, `linesUnchanged: 2`, `similarity: 0.6667`. The case runs `granularity: "word"`, and each side of the `replace` is far above `REFINE_MAX_BYTES` (16384, moves spec 3.4), so the op gets `inline: []` and `warnings` holds exactly one `refine-skipped-limit` entry with `side: null`.
