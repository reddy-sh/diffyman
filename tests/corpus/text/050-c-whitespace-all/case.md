# 050-c-whitespace-all

C code reformatted by a code formatter (spaces after commas and around operators, tab indentation) with `ignore.whitespace: "all"`, which removes every U+0009, U+000B, U+000C and U+0020 from the comparison key. All reformatted lines compare equal; only L8 also changed its tokens (` + 0` appended). A correct result has one hunk: equal L5-7, `replace` L8/R8, equal L9; `linesUnchanged: 8`, `similarity: 0.8889`.
