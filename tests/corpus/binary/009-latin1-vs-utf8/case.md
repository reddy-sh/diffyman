# 009-latin1-vs-utf8

The inputs of `text/021-invalid-utf8` under the default `adapter: "auto"`: the right file holds Latin-1 bytes, so it is binary, and the pair is compared as bytes with the `binary-fallback` warning instead of giving an `invalid-utf8` error. The left side keeps `encoding: "utf-8"`, the right side has `encoding: null` and `mediaType: "application/octet-stream"`.
