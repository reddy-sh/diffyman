# 066-char-utf16-astral

Characters outside the Basic Multilingual Plane, which take two UTF-16 code units: mathematical italic letters, a CJK Extension B ideograph and emoji, with `granularity: "char"` and `positionEncoding: "utf-16"`. A correct result has one `replace` of L1-3/R1-3 with grapheme spans of two units for each changed astral character (`𝑥`/`𝑦` at units 3 and 9 of L1/R1, `🙂`/`🙃` at unit 6 of L3/R3) and one unit for `太`/`次` at unit 6 of L2/R2 (after `𠀋`, which counts two).
