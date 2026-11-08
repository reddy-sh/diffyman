# 011-forced-text-on-binary

The PNGs of case 001 with `adapter: "text"`. Neither is UTF-8, so a correct result is a `kind: "text"` error document with `invalid-utf8` for both sides, `stats: null`, `hunks: []` and `bytes: []`; `mediaType` still says `image/png`.
