# 003-truncated

The 4096 random bytes of case 002 against their first 3000 bytes, as from an interrupted download. A correct result is one `delete` range `{leftOffset: 3000, leftLength: 1096, rightOffset: 3000, rightLength: 0}`.
