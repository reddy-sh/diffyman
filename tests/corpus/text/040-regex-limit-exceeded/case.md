# 040-regex-limit-exceeded

`ignore.regex` holds `\w{32}`: 32 Unicode word characters in a row. It is a valid pattern, but its forward NFA has more than 50000 units (states plus transitions), the limit of spec 7.2.1, while `\w{31}` is within it. The unit count does not depend on the platform, so a 32-bit WASM build must give the same result as a 64-bit native one. A correct result is `status: "error"` with exactly one `limit-exceeded` error (`side: null`), `hunks: []` and `stats: null`, with both file metas filled in and the pattern echoed in `options`.
