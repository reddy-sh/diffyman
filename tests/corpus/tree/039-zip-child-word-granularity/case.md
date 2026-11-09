# 039-zip-child-word-granularity

`granularity: "word"` applies to the child documents of a tree: the modified `readme.txt` gets a text child document with a `replace` of line 1 whose spans are `setup` (left) and `bootstrap` (right) at byte 17. A correct result has that one `modified` entry and no other change.
