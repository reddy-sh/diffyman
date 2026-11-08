# 039-regex-invalid

`ignore.regex` holds a valid pattern and an invalid one (an unclosed group). The options object itself is well formed, so the API returns a document, not an error (spec 9). A correct result is `status: "error"` with exactly one `invalid-regex` error (`side: null`), `hunks: []`, `moves: []` and `stats: null`. Both file metas are filled in (`lineCount: 4` each) and both patterns are echoed in `options` in the given order.
