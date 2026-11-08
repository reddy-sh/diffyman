# 016-gzip-vs-zip

The same payload as a gzip stream and as a stored ZIP archive, both named `.dat`. Media types come from the magic bytes (spec 16.1), not the name: a correct result has `left.mediaType: "application/gzip"` and `right.mediaType: "application/zip"`, and ranges that rebuild the ZIP from the gzip file.
