# 078-smart-thai

Moves spec 3.5 `thai`: one Thai word changes (`แมว`, cat, -> `หมา`, dog) in a sentence written without spaces. Thai is a spaceless script, so `smart` refines at `char` level, where tokens are graphemes. A correct result has one `replace` of L1/R1 with two char spans per side, `แ`/`ห` at bytes `[18, 21)` and `ว`/`า` at `[24, 27)`: the unchanged `ฉันชอบ` (bytes 0-17) and the `ม` the two words share stay equal.
