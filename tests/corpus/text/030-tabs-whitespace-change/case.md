# 030-tabs-whitespace-change

Runs `ignore.whitespace: "change"` on tab-heavy lines. Every run of tabs and spaces collapses to one space and trailing whitespace is dropped, so L1 (tabs around `=`), L2 (tab versus four spaces of indent), L3 (one and two tabs versus single spaces) and L4 (trailing tab) compare equal. L5 gains a tab where there was no whitespace and L6 gains a leading tab, so both must differ. A correct result has one hunk: equal L2-4, one `replace` of L5-6/R5-6, equal L7; `linesUnchanged: 5`, `similarity: 0.7143`.
