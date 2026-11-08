# 017-zip-encrypted

Entries with the encryption flag. A correct result never decrypts them: `sha256` is `null`, `secret.bin` is `unchanged` (equal stored bytes), `changed.bin` `modified` without a child, `encrypted-entry` warnings for both sides, and status `partial`.
