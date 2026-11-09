# 026-periodic-delete-canonical

A periodic input (the two bytes 00 01, 2048 times) and the same with one period removed. Any period could be the one removed; the common prefix is set aside first (spec 17), so the answer is canonical: a correct result is one `delete` range at the end, `{leftOffset: 4094, leftLength: 2, rightOffset: 4094, rightLength: 0}`.
