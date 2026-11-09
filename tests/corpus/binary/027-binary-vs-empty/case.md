# 027-binary-vs-empty

300 random bytes against an empty file. The empty side is compatible with both adapters, so the document is binary because of the left side. A correct result is one `delete` range `{leftOffset: 0, leftLength: 300, rightOffset: 0, rightLength: 0}`, `stats.similarity: 0`, and the empty side reports `encoding: "utf-8"`.
