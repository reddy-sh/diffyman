# 038-zip-utf8-names

Zip entries with UTF-8 names (general purpose flag bit 11): Chinese folder and file names and an accented folder name. A correct result keeps the names as UTF-8 without a `non-utf8-path` warning: `文档/说明.txt` is `modified` with a text child, `café/carte.txt` is `moved` from `café/menu.txt` (same content), and the folders `文档` and `café` are `unchanged`.
