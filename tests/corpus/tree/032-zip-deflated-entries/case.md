# 032-zip-deflated-entries

Two zips whose entries are deflate-compressed (method 8). Entries are inflated in memory and compared by their content, not their compressed bytes. A correct result has `notes.txt` `modified` with a text child document that has one `replace` at line 40, `zeros.bin` `unchanged` (4096 zero bytes on both sides), and `added.txt` `added`.
