# 008-empty-vs-binary

An empty file against 64 random bytes. An empty input is compatible with both adapters, so the document is binary because of the right side, and there is no `binary-fallback` (the empty side is not text). A correct result is one `insert` range `{leftOffset: 0, leftLength: 0, rightOffset: 0, rightLength: 64}` and `stats.similarity: 0`.
