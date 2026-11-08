# 014-nfc-vs-nfd

The left file is in NFC. On the right, lines 1, 3 and 5 are in NFD (e plus U+0301, A plus U+030A, and Hangul syllables split into conjoining jamo); lines 2 and 4 are byte-identical. The engine never normalizes (spec 7.1), so the lines look the same but must still differ: a correct result has one hunk with `replace` ops at L1, L3 and L5 and equal L2 and L4, `linesUnchanged: 2`, `status: "different"`.
