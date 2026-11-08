# 016-zip-duplicates

Duplicate paths: the same name twice, `./b.txt` and `b.txt`, and a file `x` with an entry below it. A correct result keeps the later entry of each path (`a.txt` and `b.txt` are `unchanged`), keeps `x` as a file (a `typeChange` against the implied folder on the right), and gives a `duplicate-path` warning for the left side.
