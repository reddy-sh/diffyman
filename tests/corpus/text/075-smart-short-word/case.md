# 075-smart-short-word

Moves spec 3.5 `short-word`: `foo bar` -> `foo baz`. The changed words have 3 graphemes, fewer than `SMART_SUBWORD_MIN_GRAPHEMES` (4), so the sub-word pass does not run. A correct result has one `replace` of L1/R1 with whole-word spans `bar` / `baz` at `[4, 7)`.
