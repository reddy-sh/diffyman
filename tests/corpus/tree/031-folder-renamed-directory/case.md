# 031-folder-renamed-directory

A folder renamed from `src` to `app` with its three files unchanged. Folders are not matched by content, only files are: a correct result has `src` and `src/data` `removed`, `app` and `app/data` `added`, and the three files `moved` (`app/main.py` from `src/main.py`, `app/helpers.py` from `src/helpers.py`, `app/data/table.bin` from `src/data/table.bin`), with no child documents; `setup.cfg` is `unchanged`.
