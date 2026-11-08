# 014-close-and-far-changes

2000 random bytes with single bytes flipped at offsets 100 and 104 (3 equal bytes between), 300 and 307 (6 between), 600 and 608 (7 between), and 900 and 909 (8 between). Ranges less than 8 equal bytes apart are joined (spec 17 step 4), so a correct result is five `replace` ranges: 100-104 (5 bytes), 300-307 (8 bytes), 600-608 (9 bytes), and 900 and 909 (1 byte each, kept apart).
