# 020-wasm-section-inserted

A WebAssembly module (`\0asm` magic) that gained a 15-byte custom `name` section between its type and code sections. A correct result is `mediaType: "application/wasm"` on both sides and one `insert` range of 15 bytes at left offset 15 (after the 8-byte header and the 7-byte type section), with `stats.bytes.added: 15`.
