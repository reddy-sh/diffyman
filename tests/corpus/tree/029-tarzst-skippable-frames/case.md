# 029-tarzst-skippable-frames

A zstd-compressed tar that starts with a skippable frame and has more of them between and after its two frames, against the same tar uncompressed. A correct result recognizes the left input as a tar.zst from what follows the skippable frames (spec 19.1) and is `identical`, entry for entry.
