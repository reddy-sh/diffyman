# 076-smart-two-words

Moves spec 3.5 `two-words`: `I ate a red apple` -> `I ate a green pear`. Two changed words in a row form one change region with more than one token, so the sub-word pass does not apply, and the two spans of each side, one space apart, are merged. A correct result has one `replace` of L1/R1 with spans `red apple` `[8, 17)` (left) and `green pear` `[8, 18)` (right).
