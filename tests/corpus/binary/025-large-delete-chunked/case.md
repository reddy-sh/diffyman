# 025-large-delete-chunked

192 KiB of random bytes with 70000 bytes cut out at offset 50000. The inputs are over the byte-alignment limit, so the chunked path finds the cut. A correct result is exactly one `delete` range of 70000 bytes near offset 50000, `stats.bytes.removed: 70000`, and no `bytes-coarse-limit`.
