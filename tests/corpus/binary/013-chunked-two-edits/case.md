# 013-chunked-two-edits

256 KiB of random bytes with 32 bytes inserted at offset 60000 and 40 bytes changed at offset 200000. What lies between the edits is larger than the byte-alignment limit, so the inputs are chunked (spec 17 step 3) and only the changed chunks are aligned byte by byte. A correct result is exactly two ranges, an `insert` of 32 bytes and a `replace` of 40 bytes at left offset 200000 (right offset 200032), with no `bytes-coarse-limit`.
