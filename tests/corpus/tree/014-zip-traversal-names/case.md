# 014-zip-traversal-names

Entry names that are absolute, start with a drive letter or climb with `..`. A correct result lists them as `removed` entries with paths as given after normalization (`../evil.txt`, `/etc/passwd`, `C:/Windows/win.ini`, `a/../../b.txt`, and their implied parents such as `..` and `/etc`), never resolved, with an `unsafe-path` warning for the left side.
