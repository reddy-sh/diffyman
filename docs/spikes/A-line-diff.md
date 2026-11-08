# Spike A: line diff core (`similar` vs `imara-diff`)

- **Date:** 2026-11-07
- **Question:** Which Rust line diff library and algorithm should back `diffyman-algo`, and does the Phase 1
  budget hold: a 1 GB log pair diffed in ≤60 s with ≤2 GB peak memory on the development Mac?
- **Code:** [`spikes/line-diff-bench`](../../spikes/line-diff-bench) (a standalone Cargo project)
- **Raw data:** [`results/2026-11-07-m4max.jsonl`](../../spikes/line-diff-bench/results/2026-11-07-m4max.jsonl),
  with the full per-case tables in [`results/2026-11-07-m4max.md`](../../spikes/line-diff-bench/results/2026-11-07-m4max.md)
- **Decision:** [ADR 0006](../adr/0006-line-diff-library.md)

## Summary

1. **The budget holds with a wide margin.** On the 1 GB log pair, Histogram from `gix-imara-diff` 0.3.3
   takes **1.5 s** end to end with a **400 MB** private memory footprint. That is about 40 times under the
   time budget and 5 times under the memory budget.
2. **The imara-diff engine is the fastest and leanest**, at 2 to 3 times faster than `similar` and with
   5 to 8 times less memory on large inputs. Interning the lines takes about 75% of the wall time; the
   diff itself takes 0.13 to 0.32 s for 19 M lines.
3. **Upstream `imara-diff` 0.2.0 is not usable as is.** On a file made mostly of one repeated line, all
   of its algorithms, Myers included, went past the 120 s timeout. The cause is a quadratic window scan in
   its "common line" pruning. Its `MyersMinimal` is also not minimal (+6% on logs). Both are fixed in
   **`gix-imara-diff` 0.3.3**, gitoxide's maintained copy (0.5 s on the same file, and truly minimal output).
4. **Every Histogram implementation is quadratic on one adversarial input.** On 200 k unique lines with
   each adjacent pair swapped, `gix-imara-diff` takes 33 s, `similar` 103 s and git 57 s, while every
   Myers variant takes about 0.4 s. On low-frequency repeated lines, Histogram is fast but produces twice
   the minimal edit count. The default algorithm therefore needs a deterministic work budget with a
   Myers fallback.
5. **`similar`'s wall-clock deadline is not a usable guard.** When the deadline hits near the top of the
   recursion, the output collapses into one hunk that replaces the whole file (19.3 M lines on the 1 GB
   pair). The output then depends on machine speed, which breaks our determinism rule.
6. **`similar` stays for Patience.** Neither imara-diff build has Patience. `similar`'s Patience gives
   minimal output on logs and code, and it degrades gracefully on adversarial inputs (0.4 s), but it uses
   3.4 GB on the 1 GB pair, so it needs a size limit.

## Machine and method

| Item | Value |
|---|---|
| Machine | Apple M4 Max (14 cores), 36 GB RAM, macOS 27.0, APFS SSD |
| Toolchain | rustc 1.99.1 stable (aarch64-apple-darwin) |
| Build | `--release`, `lto = "thin"`, `codegen-units = 1`, `debug = false` |
| Crates (exact pins) | `similar` 3.2.1, `imara-diff` 0.2.0, `gix-imara-diff` 0.3.3, `memmap2` 0.9.12, `memchr` 2.8.3, `hashbrown` 0.17.1 |
| Baseline | `git diff --no-index -U0 --diff-algorithm={histogram,myers}`, git 2.54.0, with `core.bigFileThreshold=8g` (without it, git reports files over 512 MiB as binary) |

**Pipeline (the same for every library):**
1. Map both files with `mmap` (`--io read` loads them into memory instead, for comparison).
2. Strip the common prefix and suffix at line granularity on the raw bytes (as GNU diff does).
3. Intern the remaining lines (newline included) into dense `u32` tokens. One `hashbrown` table maps
   line bytes to an id and is shared by both sides.
4. Run the algorithm on the token slices: `similar::algorithms::diff_deadline` over `&[u32]`, or
   `Diff::compute_with` for imara-diff and `gix-imara-diff` (their `Token` is `repr(transparent)` over `u32`).
   `imara-histogram-native` instead uses imara-diff's own `InternedInput` plus `postprocess_lines`.
5. Normalize the output to a canonical hunk list (maximal runs of non-equal lines), then verify it. The
   check confirms that every region outside the hunks matches token for token (the "Valid" column).
   It also computes an FNV fingerprint, so identical output can be spotted across algorithms.

**Measurement:**
- `run-all` runs each (case, algorithm) pair in its own process under `/usr/bin/time -l`, so the
  memory numbers are per case. Both files are read once beforehand, so the page cache is warm.
- **Two memory numbers.** *Max RSS* counts mapped file pages that were touched; the 1 GB pair adds about
  2 GB of clean, reclaimable file cache to it. *Peak footprint* (macOS `phys_footprint`, which Activity
  Monitor shows) counts the process's own memory. We judge the budget on footprint, and on max RSS for
  `--io read` rows, where the two are the same.
- **Minimality proxy.** "Del+Ins" is deleted plus inserted lines. "vs best" compares it with the smallest
  valid script seen for the case, which is the exact minimum wherever raw Myers or `gix-myers-minimal`
  finished.
- **Limits.** A hard 120 s timeout applies per run. Rows marked "+dl5s" pass a 5 s `similar` deadline.
- **Repeats.** The key 1 GB rows were run 3 times and the median is reported; they agreed within about
  10%. Other rows are single runs. The machine was not dedicated to the benchmark, so the git baseline
  in particular varied (6.8 to 17.6 s on the 1 GB pair).

## Corpus

Generated by `gen` with fixed seeds (xorshift64*, no `rand` crate) into `~/diffyman-data/corpus/spike-a`.
It is byte-for-byte reproducible and about 3 GB in total.

| Case | Size per side | What it is |
|---|---|---|
| `log-1g` | 1000 MB, 9.65 M lines (8.4 M distinct) | Timestamped service log: levels, services, request ids, HTTP/db/cache messages, Java stack traces (repeated frames). Right side: 48,331 changed lines (0.5%), 8,905 deleted in 90 blocks, 10,227 inserted in 112 blocks, 1,042 lines moved in 9 blocks, 36,240 lines with timestamp drift in 14 regions |
| `log-100m` | 100 MB, 0.96 M lines | Same generator: 4,825 changed, 6 deleted and 7 inserted blocks, 2 moves, 1 drift region |
| `source-50m` | 50 MB, 1.8 M lines, 10,443 files | Concatenated generated Rust-like files. Right side: 848 files edited, 318 functions inserted and 296 deleted, an identifier renamed throughout 96 files, 42 files moved, 27 files deleted and 28 inserted |
| `identical` | 90 MB, 5 M lines (1,039 distinct) | One repeated line. Right side: 1,038 lines replaced by unique ones, plus 1,014 extra copies of the repeated line |
| `histo-lowfreq` | 5 MB, 200 k lines (12,500 distinct) | Each line occurs 16 times (below the 63/64 Histogram cut-off), in shuffled order. The right side swaps every adjacent pair |
| `swapped-unique` | 5 MB, 200 k lines (all distinct) | All lines unique; the right side swaps every adjacent pair |
| `huge-line` | 50 MB | One 50 MB line between short lines; the right side changes 8 bytes in it |
| `all-different` | 44 MB, 1 M lines | No line is shared between the two sides |

## Results

Wall time is end to end (load, trim, intern, diff, verify). Memory is in MB. The full tables, with every
`similar` deadline variant, `imara-diff` 0.2.0 rows and intern/diff split, are in the
[raw report](../../spikes/line-diff-bench/results/2026-11-07-m4max.md).

### 1 GB log pair (the budget case)

| Algorithm | Wall s | Diff s | Footprint MB | Max RSS MB | Hunks | Del+Ins | vs best |
|---|--:|--:|--:|--:|--:|--:|--:|
| **gix-histogram** (median of 3) | **1.48** | 0.32 | **400** | 2307 | 48,485 | 196,662 | +3.5% |
| gix-myers (median of 3) | 1.28 | 0.13 | 510 | 2417 | 48,213 | 200,232 | +5.4% |
| gix-myers-minimal (median of 3) | 1.32 | 0.25 | 509 | 2416 | 48,874 | 189,980 | 0.0% |
| imara-histogram 0.2.0 (median of 3) | 1.41 | 0.30 | 400 | 2307 | 48,485 | 196,662 | +3.5% |
| imara-histogram, own interner and `postprocess_lines` | 1.68 | 0.29 | 480 | 2387 | 48,480 | 196,662 | +3.5% |
| imara-histogram with `--io read` | 2.00 | 0.32 | 2307 | 2306 | 48,485 | 196,662 | +3.5% |
| similar-myers (median of 3) | 2.30 | 1.26 | 2426 | 3877 | 48,206 | 200,350 | +5.5% |
| similar-patience (median of 3) | 3.97 | 2.83 | 3400 | 5104 | 48,861 | 189,980 | 0.0% |
| similar-histogram (median of 3) | 4.12 | 3.00 | 2856 | 4762 | 48,858 | 190,024 | +0.0% |
| similar-myers-raw (exact) | 72.00 | 70.93 | 289 | 2193 | 48,873 | 189,980 | 0.0% |
| similar-myers-raw, 5 s deadline | 6.05 | 5.00 (hit) | 286 | 2193 | **1** | 19,297,348 | +10,058% |
| similar-histogram, 5 s deadline | 6.11 | 5.00 (hit) | 2856 | 4761 | 48,597 | 294,528 | +55% |
| git histogram (median of 3, writes patch text) | 14.97 | – | 1832 | 3740 | 48,860 | 189,990 | +0.0% |
| git myers (writes patch text) | 6.11 | – | 1400 | 3307 | 48,209 | 200,232 | +5.4% |

**Budget verdict:** met. The default candidate uses 1.5 s and 400 MB of private memory. Max RSS is
2.3 GB only because the 2 GB of mapped input pages count towards it. Loading the inputs into memory
instead (`--io read`) pushes the footprint to 2.3 GB, so the large-file mode must use mmap.

### Other realistic inputs

| Case | Algorithm | Wall s | Footprint MB | Hunks | Del+Ins | vs best |
|---|---|--:|--:|--:|--:|--:|
| log-100m | gix-histogram | 0.27 | 43 | 4,833 | 18,954 | +3.9% |
| log-100m | gix-myers-minimal | 0.10 | 65 | 4,877 | 18,234 | 0.0% |
| log-100m | similar-histogram | 0.31 | 201 | 4,877 | 18,234 | 0.0% |
| log-100m | similar-patience | 0.27 | 263 | 4,877 | 18,234 | 0.0% |
| log-100m | similar-myers | 0.16 | 178 | 4,813 | 19,258 | +5.6% |
| source-50m | gix-histogram | 0.26 | 52 | 2,745 | 38,237 | +0.2% |
| source-50m | gix-myers | 0.15 | 85 | 2,993 | 38,175 | 0.0% |
| source-50m | similar-patience | 0.26 | 136 | 2,775 | 38,175 | 0.0% |
| source-50m | similar-histogram | 1.84 | 138 | 2,738 | 38,251 | +0.2% |
| source-50m | similar-myers | 0.21 | 42 | 3,022 | 38,409 | +0.6% |
| source-50m | git histogram | 0.53 | 232 | 2,745 | 38,237 | +0.2% |

On source code, all algorithms are within 0.6% of the minimum. Histogram and Patience produce the fewest
hunks (2,738 to 2,775, against 2,904 to 3,123 for the library Myers variants; git's Myers, which compacts
hunks afterwards, gets 2,774), which is the readability gain they are chosen for.
`gix-histogram` gives the same counts as git's histogram here.

### Pathological inputs (wall seconds; "T/O" means over 120 s)

| Algorithm | identical | histo-lowfreq | swapped-unique | huge-line | all-different |
|---|--:|--:|--:|--:|--:|
| gix-histogram | 0.48 | 0.05 (**+98%**) | **33.2** | 0.05 | 0.15 |
| gix-myers | 0.52 | 0.43 | 0.43 | 0.05 | 0.16 |
| gix-myers-minimal | 0.21 | 23.8 | 23.8 | 0.05 | 0.16 |
| imara-histogram 0.2.0 | **T/O** | 0.05 (+98%) | 32.8 | 0.05 | 0.16 |
| imara-myers 0.2.0 | **T/O** | 0.26 | 0.22 | 0.05 | 0.16 |
| imara-myers-minimal 0.2.0 | **T/O** | 24.3 | 24.3 | 0.05 | 0.16 |
| similar-myers | 0.16 | 0.37 | 0.41 | 0.05 | 0.26 |
| similar-myers-raw | 0.16 | 62.4 | 47.6 | 0.05 | **T/O** |
| similar-myers-raw, 5 s deadline | 0.16 | 5.05 (+100%) | 5.05 (+100%) | 0.06 | 5.16 |
| similar-patience | 0.21 | 0.37 | 0.47 | 0.05 | 0.21 |
| similar-histogram | 0.26 | 4.72 (+35%) | **102.5** | 0.05 | 0.21 |
| similar-histogram, 5 s deadline | 0.32 | 5.06 (+54%) | 5.06 (+99%) | 0.05 | 0.21 |
| git histogram | 0.95 | 0.21 (+98%) | 57.2 | 0.47 | 0.78 |
| git myers | 0.80 | 0.58 | 0.58 | 0.48 | 0.73 |

Percentages show the edit count above the minimum, where it is not minimal. All library runs on
pathological inputs stayed under 300 MB of footprint. All scripts that finished passed the validity check.

## Findings in detail

1. **Interning dominates on large files.** On the 1 GB pair, interning takes 1.0 to 1.1 s and the diff
   0.13 to 0.32 s. The interner is the place to optimize: intern the two sides on two threads, or hash
   with a faster function. imara-diff's own `InternedInput` path is about 0.3 s slower than ours and uses
   80 MB more, so `diffyman-algo` should own the interner.
2. **imara-diff 0.2.0 common-line pruning is quadratic.** In `myers/preprocess.rs`,
   `should_prune_common_line` starts its backward window at index 100 instead of `pos - 100`, so it scans
   the whole prefix for every common line. Files made mostly of one line (blank lines, separators,
   heartbeat logs) hit this, and Histogram is affected too because it falls back to Myers for lines
   occurring more than 63 times. `gix-imara-diff` 0.3.3 fixes the window. It also counts tokens before
   trimming, as git does, and disables pruning for `MyersMinimal`. As a result its minimal mode really
   is minimal: 189,980 edits on the 1 GB pair, against 201,530 for upstream.
3. **Histogram's worst case is real and shared with git.** If every common run has length 1 and anchors
   land at the edge of the region, each step rebuilds the histogram for almost the whole region:
   O(N²). 200 k lines already take 33 s with `gix-imara-diff`, so a 2 M-line file would take about an hour. No
   library offers a work limit. `similar`'s deadline stops it, but at the cost of the output quality
   (+99%) and of determinism.
4. **Histogram quality can also collapse.** On `histo-lowfreq` (every line 16 times, pairs swapped),
   imara/gix and git Histogram produce one near-total replacement (1,760 hunks, +98%), while every Myers
   finds the 199,962-edit minimum in under 0.5 s.
5. **`similar`'s heuristic Myers and Patience are robust but heavy.** They never went past 0.5 s on
   pathological inputs, but use 2.4 to 3.4 GB on the 1 GB pair. Raw Myers is exact but O(ND): 72 s on
   the 1 GB pair, and it timed out on `all-different`, because the raw variant skips the "disjoint
   ranges" fast path on purpose.
6. **`similar`'s deadline degrades badly.** On a deadline, the unfinished region becomes one
   delete-plus-insert hunk. When that happens at the top level, the entire file is reported as replaced.
7. **No two libraries give identical scripts on logs.** The only exact matches are `gix-imara-diff` and
   `imara-diff` 0.2.0 Histogram (same fingerprints on every case that finished), and `gix-myers-minimal`
   with `similar` Patience/Histogram on `log-100m`. An algorithm ID must therefore name the library and
   its exact version, not just the algorithm.
8. **Hook convention detail.** On replacements, `similar` can report an insert at the *start* of the
   range it just deleted (for example `delete(s, n, t)` followed by `insert(s, t, m)`). An adapter that
   expects the insert at `s + n` builds invalid hunks. The bench normalizes this, and so must
   `diffyman-algo`.
9. **Huge lines are cheap.** A 50 MB line is hashed once (about 4 ms). The line diff is trivial;
   the real cost is in the word and character refinement, which the engine limits to small hunks
   (see [text granularity and moves](../spec/text-granularity-and-moves.md)).
10. **Moved blocks** show up in every library as a separate delete and insert, as expected. Move
    detection stays our own post-pass over the line diff (see
    [the moved-blocks spec](../spec/text-granularity-and-moves.md#4-moved-blocks)).

## Conclusion

- Use **`gix-imara-diff` 0.3.3** (Apache-2.0, maintained by the gitoxide project) for `text.histogram@1`
  (the default) and `text.myers@1`. Do not depend on `imara-diff` 0.2.0.
- Use **`similar` 3.2.1** for `text.patience@1`, with an input size limit.
- Own the front end in `diffyman-algo`: mmap, prefix/suffix trim, interning and hunk normalization.
- Guard Histogram with a **deterministic work budget** and a Myers fallback. This needs our own copy of
  the Histogram module (about 500 lines, Apache-2.0) or an upstream patch. Do not use wall-clock
  deadlines to shape output; use them only as a last-resort `limit_exceeded` error.
- Keep the Phase 1 budget (1 GB in ≤60 s, ≤2 GB) and measure memory as peak footprint. Today's headroom
  (1.5 s, 0.4 GB) leaves room for word-level refinement, move detection and JSON output.

Details and consequences are in [ADR 0006](../adr/0006-line-diff-library.md).

## Reproduce

```zsh
scripts/with-rustup cargo build --release --manifest-path spikes/line-diff-bench/Cargo.toml
spikes/line-diff-bench/target/release/gen ~/diffyman-data/corpus/spike-a
spikes/line-diff-bench/target/release/run-all --data ~/diffyman-data/corpus/spike-a \
  --out spikes/line-diff-bench/results/run.jsonl --timeout-s 120
spikes/line-diff-bench/target/release/run-all --report spikes/line-diff-bench/results/run.jsonl
```

The full run takes roughly half an hour, most of it in the runs that hit the timeout and in the
quadratic Histogram and exact-Myers rows.
