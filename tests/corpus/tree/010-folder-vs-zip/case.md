# 010-folder-vs-zip

A folder against a zip of the same files; the zip has no directory entries. A correct result is `identical`: `sub` is implied by `sub/b.txt` on the right, the right side has `mediaType: "application/zip"` and the zip's own size and SHA-256.
