# 002-inserted-bytes

4096 random bytes, and the same bytes with 16 bytes inserted at offset 2048. Byte alignment must find the insertion rather than report everything after it as changed: a correct result has one `insert` range with `leftLength: 0` and `rightLength: 16`, and `stats.bytes` with `added: 16`, `removed: 0`, `unchanged: 4096`.
