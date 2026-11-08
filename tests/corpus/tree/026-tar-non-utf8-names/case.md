# 026-tar-non-utf8-names

A tar entry named in Latin-1. A correct result decodes it as `caf\ufffd.txt` with a `non-utf8-path` warning for the left side; its content equals `café.txt` on the right, so the pair is one `moved` entry.
