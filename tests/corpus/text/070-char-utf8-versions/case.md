# 070-char-utf8-versions

Two dependency version bumps in a TOML file, with `granularity: "char"` and `positionEncoding: "utf-8"`: characters are graphemes, so a span can cover one digit. A correct result has one `replace` of L2-3/R2-3 with character spans on the changed digits only (`19`/`28` at the end of the serde version, and the two changed digits of the regex version), and equal L1 and L4.
