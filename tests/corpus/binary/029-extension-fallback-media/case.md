# 029-extension-fallback-media

Binary data named `.jpg` that does not start with a JPEG signature (it starts with a NUL byte). No magic number matches, so the binary entry of the extension table gives the media type: a correct result has `mediaType: "image/jpeg"` on both sides and one `replace` range of 1 byte at offset 2000.
