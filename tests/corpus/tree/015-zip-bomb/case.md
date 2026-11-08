# 015-zip-bomb

A tiny zip bomb: 2 MiB and 3 MiB of zeros deflate to about 13 and 20 KB, over the ratio limit of 100. A correct result never inflates them: their `sha256` is `null` and `size` is the declared size, `zeros.bin` is `unchanged` (equal stored bytes), `more-zeros.bin` is `added`, `readme.txt` is `modified` with a child, there are `tree-limit` warnings for both sides, and the status is `partial`.
