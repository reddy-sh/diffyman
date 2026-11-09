# 018-gif-palette-edit

A GIF (`GIF89a` magic, logical screen descriptor, a 256-colour palette and image bytes) whose palette entry 5 changed colour: three bytes at offset 28. A correct result is `mediaType: "image/gif"` on both sides (by magic) and exactly one `replace` range `{leftOffset: 28, leftLength: 3, rightOffset: 28, rightLength: 3}`.
