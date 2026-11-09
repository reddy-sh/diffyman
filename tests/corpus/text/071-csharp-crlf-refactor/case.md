# 071-csharp-crlf-refactor

A C# class with CRLF line endings on both sides: a constant is introduced and two methods become expression-bodied. Both sides use CRLF throughout, so the terminator never makes a line differ. A correct result has one hunk: equal L1-2, `insert` R3, equal L3-4, `replace` L5-6/R6-7, equal L7-9; both sides `lineTerminator: "crlf"`, `linesAdded: 3`, `linesRemoved: 2`, `linesUnchanged: 7`.
