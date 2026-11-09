# 080-smart-insert-only

Moves spec 3.5 `insert-only`: a pure insertion with `granularity: "smart"`. Only `replace` ops are refined, so a correct result has one hunk with equal L1, an `insert` of R2 with `inline: []`, and equal L2/R3; no warnings.
