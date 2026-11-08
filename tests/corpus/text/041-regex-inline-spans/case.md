# 041-regex-inline-spans

Two backup logs with new timestamps and durations, masked by `ignore.regex`. L3 differs only in masked text, so it is equal; L1 and L2 form one `replace` op. Inline refinement applies the same masks (moves spec 3.2): each match is one token equal to any match of the same pattern, so the timestamps and `in 256 s` / `in 342 s` get no span. The only word spans are the volume number (`01` / `02`) and the skipped count (`14` / `9`).
