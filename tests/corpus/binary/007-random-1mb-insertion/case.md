# 007-random-1mb-insertion

1 MiB of random bytes, and the same bytes with 100 random bytes inserted at offset 400000. A correct result is exactly one `insert` range with `rightLength: 100` near offset 400000 (common prefix and suffix are set aside first), `stats.bytes.unchanged: 1048576` and no `bytes-coarse-limit`.
