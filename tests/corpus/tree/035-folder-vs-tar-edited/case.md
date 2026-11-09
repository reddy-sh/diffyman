# 035-folder-vs-tar-edited

A folder against a tar of the same files with one line edited. The tar lists `docs/` explicitly and implies `src`; the folder has both as real folders. A correct result has `docs/guide.txt` `modified` with a text child document (one `replace` of line 2), and `docs`, `src` and `src/app.js` `unchanged`; the right side has `mediaType: "application/x-tar"`.
