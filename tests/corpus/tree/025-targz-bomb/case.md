# 025-targz-bomb

A gzip-compressed tar holding 4 MiB of zeros in about 26 KB. A correct result stops reading the left stream once it passes the ratio limit: only `first.txt` is listed (`unchanged`), the left side has `tree-limit`, and the status is `partial`.
