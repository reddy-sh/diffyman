# 018-log-timestamps-regex

Two runs of the same job log; every timestamp differs, and so do the pids, one latency value and the batch 3 outcome. `ignore.regex` holds one pattern that masks the leading ISO 8601 timestamp (spec 7.2.1), so lines that differ only in their timestamp compare equal. A correct result is `different` with one hunk covering L1-10: `replace` L1-2 (the pids), equal L3, `replace` L4 (the latency), equal L5-6, `replace` L7 (the batch 3 outcome) and equal L8-10; `linesUnchanged: 6`, `similarity: 0.6`, and the pattern echoed in `options`.
