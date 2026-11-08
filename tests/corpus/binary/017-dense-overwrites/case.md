# 017-dense-overwrites

128 KiB of random bytes with 30 runs of 8 bytes overwritten, one every 4228 bytes. The edits are closer together than the average first-level chunk (8 KiB), so every first-level chunk changes and the changed region is far over the byte-alignment limit; the smaller chunk levels (spec 17 step 3) still align it. A correct result is exactly 30 `replace` ranges of 8 bytes at left offsets 4228, 8456, ... 126840 (equal right offsets), `stats.bytes.removed: 240`, and no `bytes-coarse-limit`.
