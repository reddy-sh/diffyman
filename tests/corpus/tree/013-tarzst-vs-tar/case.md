# 013-tarzst-vs-tar

A zstd-compressed tar against a plain one, with pax and GNU long names, links and a FIFO. A correct result reads the 154-byte pax path and the GNU long name, lists `latest` as an `unchanged` symlink, `copy.js` as a `modified` hardlink, `pipe` as `other`, and the edited files as `modified` with text children.
