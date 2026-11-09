# 077-smart-cjk

Moves spec 3.5 `cjk`: `我喜欢猫` -> `我喜欢狗`. Every grapheme is Han, a spaceless script, so `smart` refines at `char` level (moves spec 3.3 step 1). A correct result has one `replace` of L1/R1 with char spans `猫` / `狗` at bytes `[9, 12)`.
