# 034-smart-noise

Runs `granularity: "smart"` where line 2 is replaced by an unrelated sentence. No word is shared, so almost every non-whitespace grapheme lies in a span and the smart noise cutoff (moves spec 3.3 step 4, 25 %) applies. A correct result has one hunk: equal L1, `replace` L2/R2 with `inline: []`, equal L3. The cutoff is not a size limit, so there is no `refine-skipped-limit` warning and `warnings` is empty.
