# 032-codepoint-char-combining

Runs `granularity: "char"` with `positionEncoding: "codepoint"` on combining marks. Tokens are extended grapheme clusters, so a span never splits a base letter from its marks. L1 swaps U+0301 for U+0300 after `e`: spans `[3, 5)` on both sides. L2 is precomposed U+00F1 on the left and `n` plus U+0303 on the right, which look the same but differ (no normalization, spec 7.1): left `[2, 3)`, right `[2, 4)`. L3 drops U+0323 from a stacked cluster: left `[0, 3)`, right `[0, 2)`. A correct result has one `replace` of L1-3/R1-3 holding exactly these six spans.
