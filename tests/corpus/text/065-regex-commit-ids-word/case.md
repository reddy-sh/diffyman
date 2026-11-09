# 065-regex-commit-ids-word

A changelog whose commit ids all changed after a rebase, with `ignore.regex` masking seven-digit hexadecimal ids and `granularity: "word"`. L3 differs only in its masked id, so it is equal; L2 and L4 form two `replace` ops. Masked ids get no span (moves spec 3.2), so a correct result has word spans only on `folder`/`directory` (L2/R2) and `lazily`/`on demand` (L4/R4).
