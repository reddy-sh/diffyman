# 015-emoji-zwj

Emoji built from several code points: a ZWJ profession sequence, a four-person ZWJ family, a skin-tone modifier, a regional-indicator flag and a variation selector. On the right, L1 swaps the woman for a man inside the ZWJ sequence, L3 changes the skin tone, L4 changes the flag (JP to JM) and L6 drops U+FE0F. L2 and L5 are unchanged. A correct result has one hunk: `replace` L1, equal L2, `replace` L3-4, equal L5, `replace` L6; `linesUnchanged: 2`.
