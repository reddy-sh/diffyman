# 036-zip-vs-targz-identical

The same two files as a zip (one entry deflated, one stored) and as a gzip-compressed tar. Archive formats and compression do not matter, only paths, types and content. A correct result is `identical`: `a.txt`, `sub` (implied by `sub/b.bin` on both sides) and `sub/b.bin` `unchanged`, with `application/zip` and `application/gzip` media types.
