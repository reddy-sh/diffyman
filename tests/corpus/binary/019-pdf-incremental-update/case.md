# 019-pdf-incremental-update

A PDF and the same file after an incremental update, which appends a new object and trailer and never rewrites earlier bytes. The binary comment on line 2 and the stream make it binary. A correct result is `mediaType: "application/pdf"` on both sides and one `insert` range at the end: `{leftOffset: 711, leftLength: 0, rightOffset: 711, rightLength: 101}`.
