# 037-folder-child-ignore-whitespace

Text options apply to the child documents of a tree (spec 9, 19.5): with `ignore.whitespace: "all"`, `style.css` differs only in whitespace. Its bytes and SHA-256 differ, so the entry is `modified`, but its child document is `identical` with `hunks: []`. `notes.txt` is `modified` with a child document that has one `replace` at line 2.
