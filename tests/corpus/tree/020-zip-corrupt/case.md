# 020-zip-corrupt

A zip whose end record is cut off. Its first bytes still say zip, so the pair is a tree, and a correct result is an error document with `invalid-archive` for the left side, file meta for both sides, and `tree: null`.
