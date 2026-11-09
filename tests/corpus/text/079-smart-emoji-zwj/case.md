# 079-smart-emoji-zwj

Moves spec 3.5 `emoji`: `👩‍💻 ok` -> `👨‍💻 ok`. The ZWJ sequence is one grapheme, so a span never splits it. A correct result has one `replace` of L1/R1 with one span per side covering the whole cluster, bytes `[0, 11)` on both sides, and no span on ` ok`.
