# 062-context-zero-many-hunks

Every fourth line of a 24-line properties file changes its value, with `context: 0`. Each change op is then its own hunk and holds no equal op (spec 7.4). A correct result has six hunks, h1-h6, each a single one-line `replace` at L4, L8, L12, L16, L20 and L24, `hunkCount: 6`, `changeCount: 6` and `linesUnchanged: 18`.
