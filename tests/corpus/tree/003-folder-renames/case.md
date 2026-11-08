# 003-folder-renames

Files moved to new paths with their content unchanged. A correct result has `moved` entries at the right paths with `fromPath` set: `lib/util.py` from `src/util.py`, `img/logo.bin` from `assets/logo.bin`, and for the two files with equal content `moved/b.txt` from `notes/b.txt` (same last component first) and `archive/x.txt` from `notes/a.txt`. Empty files never move: `empty-old.txt` is `removed` and `empty-new.txt` `added`. Moved files get no child document.
