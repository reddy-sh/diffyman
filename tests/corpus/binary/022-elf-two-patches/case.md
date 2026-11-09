# 022-elf-two-patches

A shared library (`\x7fELF` magic) with two 4-byte patches far apart, as a binary patch tool writes them. A correct result has `mediaType: "application/x-executable"` from the magic on both sides (the `.so` extension is not in the table) and exactly two `replace` ranges, 4 bytes at offset 1000 and 4 bytes at offset 15000, the same offsets on both sides.
