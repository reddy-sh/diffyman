# 022-zip-forged-entry-count

A zip64 end record that claims 2^40 entries. A correct result refuses it before reading the central directory: an error document with `limit-exceeded`.
