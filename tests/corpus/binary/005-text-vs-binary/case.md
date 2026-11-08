# 005-text-vs-binary

A UTF-8 text file against a PNG. The PNG is not UTF-8, so under `adapter: "auto"` the pair is compared as bytes and gets the `binary-fallback` warning. A correct result has `left.mediaType: "text/plain"` with `encoding: "utf-8"`, `right.mediaType: "image/png"` with `encoding: null`, no line fields, and ranges that rebuild the PNG from the text.
