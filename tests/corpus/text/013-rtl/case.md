# 013-rtl

Arabic and Hebrew lines, including digits and a Latin acronym inside right-to-left text. Only a number changes on line 3 (Arabic) and line 5 (Hebrew). The diff works on logical (storage) order, never on display order. A correct result has one hunk covering L1-6 with two single-line `replace` ops at L3 and L5 and equal ops everywhere else, `linesUnchanged: 4`.
