# 001-identical-folders

Two folders with the same files in nested folders. A correct result is `kind: "tree"`, `status: "identical"`, every entry `unchanged` (directories included), equal manifest digests in `left.sha256` and `right.sha256`, `mediaType: "inode/directory"` and similarity 1.
