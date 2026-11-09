# 021-sqlite-page-overwrite

A 16 KiB SQLite database file (`SQLite format 3` magic, 4 KiB pages) in which 100 bytes of page 3 were rewritten in place. The extension `.db` is not in the table, so the media type comes from the magic: `application/vnd.sqlite3`. A correct result is one `replace` range `{leftOffset: 8292, leftLength: 100, rightOffset: 8292, rightLength: 100}`.
