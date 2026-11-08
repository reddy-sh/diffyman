# 011-tabs-vs-spaces

Runs `ignore.whitespace: "trailing"`. Line 3 only gains a trailing tab, which this mode removes, so it compares equal. Lines 2 and 5 swap four or eight leading spaces for tabs; leading whitespace is not ignored in this mode, so they must show as changed. A correct result has one hunk with two single-line `replace` ops (L2/R2 and L5/R5) separated by equal L3-4, `linesUnchanged: 5` and `status: "different"`.
