# 027-only-newline

The left file is a single LF byte; the right file is empty. `"\n"` is one empty line (spec 7.2), so the left side has `size: 1`, `lineCount: 1`, `finalNewline: true` and `lineTerminator: "lf"`, while the right side has `lineCount: 0`, `finalNewline: false` and `lineTerminator: "none"`. A correct result is `different` with one hunk holding one `delete` with `leftRange {start: 1, count: 1}` and `rightRange {start: 1, count: 0}`, `linesRemoved: 1`, `similarity: 0` and no warnings.
