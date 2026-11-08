# 022-utf16-bom

The right file is the left file's text saved as UTF-16LE with its byte order mark (FF FE). Only EF BB BF is treated as a BOM, and a UTF-16 BOM fails the UTF-8 check (spec 7.1), so a correct result is `status: "error"` with one `invalid-utf8` error for the right side and no `bom-stripped` warning. The right side reports `bom: false`, `size: 70` and `null` for `encoding`, `lineTerminator`, `finalNewline` and `lineCount`; the left side decodes normally (`lineCount: 4`). `hunks` and `moves` are `[]` and `stats` is `null`.
