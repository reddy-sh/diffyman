# 055-html-word-codepoint

An HTML page (`mediaType: "text/html"` from the extension) with attribute edits, run with `granularity: "word"` and `positionEncoding: "codepoint"`. A correct result has `replace` L2/R2 with spans on `en`/`sv`, equal L3-4, and `replace` L5-6/R5-6 with a right-only span for the added ` warning` class and spans on `length`/`size` in the link target. Punctuation such as `"`, `=` and `/` is its own segment and stays equal.
