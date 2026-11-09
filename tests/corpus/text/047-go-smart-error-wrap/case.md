# 047-go-smart-error-wrap

A Go function whose two bare `return nil, err` lines now wrap the error, run with `algorithm: "myers"` and `granularity: "smart"`. Tab-indented code. A correct result has one hunk with two single-line `replace` ops (L4/R4 and L8/R8) around equal L5-7. Each replaced line keeps `\t\treturn nil, ` and `err`, so no left span exists and the right spans cover only the inserted text, in two pieces per line: `fmt.Errorf("read %s: %w", path, ` at byte 14 and the closing `)` after `err` on R4, and the same with `parse` on R8.
