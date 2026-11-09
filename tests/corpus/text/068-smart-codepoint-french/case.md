# 068-smart-codepoint-french

French prose with `granularity: "smart"` and `positionEncoding: "codepoint"`: an accent is lost (`élève` -> `éleve`) and a word is replaced (`écrit` -> `rédigé`). A correct result has one `replace` of L1-2/R1-2. The accent change is a one-word swap of at least 4 graphemes, so the sub-word pass marks only `è`/`e` at code point 4 of L1/R1; `écrit`/`rédigé` is not similar enough and keeps whole-word spans at code point 24.
