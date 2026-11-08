# 010-utf16-vs-utf8

The inputs of `text/022-utf16-bom` under `adapter: "auto"`. The right file is valid UTF-16LE text, which the text adapter does not read, so both are compared as bytes with `binary-fallback`. A correct result reports the right side as `encoding: "utf-16le"` and `mediaType: "text/plain"`.
