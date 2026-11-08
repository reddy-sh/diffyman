# 028-zip-end-record-in-content

Both zips store, as the content of `record.bin`, the bytes of a zip64 end record that claims 2^40 entries. No locator at the end of the archive names it, so it is data, not an end record (spec 19.3). A correct result lists both entries: `record.bin` is `unchanged` and `notes.txt` is `modified` with a text child.
