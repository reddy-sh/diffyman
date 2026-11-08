# 022-zip-forged-entry-count

A zip that ends the zip64 way (zip64 end record, locator, and a zip32 end record whose fields defer to it), with a zip64 end record that claims 2^40 entries. A correct result refuses it before reading the central directory: an error document with `limit-exceeded`.
