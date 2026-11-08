# 027-zip-overlapping-entries

The left zip lists three names whose entries all point at one local entry, as an overlapping zip bomb does to inflate one compressed stream many times. The right zip holds the same content three times, normally. A correct result reads the left stream once: `a.txt` (the first entry by offset) has its `sha256` and is `unchanged`; `b.txt` and `c.txt` overlap it, so on the left they have `sha256: null` and their declared size, they are `modified` without a child document, the left side has `tree-limit`, and the status is `partial` (spec 19.6).
