# 015-unrelated-coarse

Two unrelated 48 KiB random files. Together they exceed the byte-alignment limit and share no chunk, so the changed region is reported as one `replace` range over both whole files with the `bytes-coarse-limit` warning (spec 17 step 3).
