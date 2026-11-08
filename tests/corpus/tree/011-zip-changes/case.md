# 011-zip-changes

Two zips: an edited file, an added file, a moved file and a symbolic link whose target changed. A correct result has `docs/guide.txt` `modified` with a text child, `latest` a `modified` symlink (targets `src/app.js` and `src/main.js`, never resolved), `new/name.txt` `moved` from `old/name.txt`, and `docs` `unchanged` (explicit on the left, implied on the right).
