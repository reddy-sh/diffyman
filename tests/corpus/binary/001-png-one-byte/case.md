# 001-png-one-byte

A 16x16 RGBA PNG and the same file with one pixel byte (offset 200) changed by one bit; the CRC is not updated, as in a corrupted copy. Both sides are binary (`image/png` by magic), so a correct result is `kind: "binary"` with exactly one `replace` range `{leftOffset: 200, leftLength: 1, rightOffset: 200, rightLength: 1}`, no warnings, and `stats.bytes.unchanged` one less than the size.
