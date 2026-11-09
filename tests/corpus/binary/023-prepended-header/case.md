# 023-prepended-header

6000 random bytes, and the same bytes with a 64-byte header in front. Nothing is common at the start, so the common suffix carries the match. A correct result is one `insert` range `{leftOffset: 0, leftLength: 0, rightOffset: 0, rightLength: 64}`.
