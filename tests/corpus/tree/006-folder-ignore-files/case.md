# 006-folder-ignore-files

`tree.ignoreFiles` with `.ignore` files at the root and in a subfolder (a nested file can re-include). A correct result leaves out `a.tmp` and `cache/`, lists both `.ignore` files as `unchanged`, and `src/keep.tmp` as `modified`.
