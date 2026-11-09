# 057-shell-crlf-trailing

A shell script saved with CRLF line endings and trailing spaces on two lines, plus a real change (keep 30 days of backups instead of 14). Under `ignore.whitespace: "trailing"` trailing whitespace is dropped and the terminator is not part of the key, so only the last line differs. A correct result has one hunk: equal L4-6, `replace` L7/R7; the left side reports `lineTerminator: "lf"`, the right side `"crlf"`, `linesUnchanged: 6`.
