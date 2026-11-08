# 005-folder-exclude

`tree.exclude` with a glob, a directory pattern and a negation. A correct result leaves out `app.log` and everything under `build/`, keeps `keep.log` (re-included) as `modified` with a child document, and lists `src` and `src/main.c` as `unchanged`.
