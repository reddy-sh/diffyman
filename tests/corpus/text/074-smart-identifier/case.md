# 074-smart-identifier

Moves spec 3.5 `identifier`: `getUserName()` -> `getUsername()` with `granularity: "smart"`. The word pass sees one changed word-like token on each side (`user.getUserName` / `user.getUsername`: UAX #29 joins letters across the full stop), long and similar enough for the sub-word pass. A correct result has one `replace` of L1/R1 with char spans `N` (left) and `n` (right) at byte 19, and equal L2.
