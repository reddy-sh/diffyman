# 033-tar-file-becomes-symlink

A tar where a regular file was replaced by a symbolic link to a sibling with the same content. The type differs, so a correct result has `config.ini` as `typeChange` (left `file`, right `symlink` with target `config.live.ini`, never followed) and no child document; `config.live.ini` and `logs` are `unchanged`.
