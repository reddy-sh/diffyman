# 0006. Line diff library and algorithm IDs

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

Every text, code and document diff ends in a line diff, so this choice sets most of diffyman's speed,
memory use and output quality. Two Apache-2.0 crates were considered, `similar` and `imara-diff`. The
Phase 1 budget is a 1 GB log pair in ≤60 s with ≤2 GB peak memory on the development Mac. The output must
be deterministic, and algorithm IDs are versioned (`text.histogram@1`). Hostile inputs must not hang the
engine.

[Spike A](../spikes/A-line-diff.md) measured both crates, plus `gix-imara-diff` 0.3.3 (gitoxide's maintained
copy of imara-diff) and git, on logs, a source tree and five pathological inputs. Memory below is peak
footprint:

| | gix-imara-diff 0.3.3 | imara-diff 0.2.0 | similar 3.2.1 |
|---|---|---|---|
| 1 GB log, Histogram | 1.5 s, 0.4 GB | 1.4 s, 0.4 GB | 4.1 s, 2.9 GB |
| 1 GB log, Myers | 1.3 s, 0.5 GB | 1.2 s, 0.5 GB | 2.3 s, 2.4 GB |
| 1 GB log, Patience | not available | not available | 4.0 s, 3.4 GB |
| 5 M copies of one line | 0.5 s | over 120 s (pruning bug) | 0.2 s |
| 200 k lines, pairs swapped, Histogram | 33 s (quadratic) | 33 s | 103 s |
| Work or time limit | none | none | wall-clock deadline |

When `similar`'s deadline hits, the unfinished region becomes one delete-plus-insert hunk; on the 1 GB pair
that meant a single 19.3 M-line replacement, and the result depends on machine speed. Interning lines takes
about 75% of the 1 GB run.

## Decision

1. **Libraries, pinned exactly:** `gix-imara-diff = "=0.3.3"` for Histogram and Myers, and
   `similar = "=3.2.1"` for Patience (and later `TextMerge`). Their dependencies (`bstr`, `hashbrown`,
   `foldhash`) are MIT, Apache-2.0 or Zlib.
2. **Algorithm IDs:**

   | ID | Engine |
   |---|---|
   | `text.histogram@1` (**default**) | `gix-imara-diff` Histogram with our work budget (item 3), falling back to its Myers |
   | `text.myers@1` | `gix-imara-diff` `Algorithm::Myers` (xdiff-style heuristics, bounded, near-minimal) |
   | `text.patience@1` | `similar` `Algorithm::Patience`, no deadline, inputs ≤256 MB per side by default |

   An ID pins the library, its exact version and our own parameters. Any change that can alter output gets
   `@2` and a changelog entry. A minimal Myers ID is deferred.
3. **Deterministic Histogram guard.** Histogram is quadratic on some inputs in every implementation,
   git included, and no crate has a work limit. Phase 1 keeps a copy of the `gix-imara-diff` Histogram
   module (about 500 lines, Apache-2.0, header and `NOTICE` kept) with an operation counter. Past a
   size-proportional budget, the rest of the region is diffed with Myers and an `algorithm_fallback`
   warning is set. Acceptance tests: the 200 k-line swap case runs in ≤1 s, and output on normal inputs is
   byte-identical to unguarded Histogram. We will offer the counter upstream.
4. **Shared front end in `diffyman-algo`:** mmap input, line-granular prefix and suffix trim, one `u32`
   interner, and hunk normalization that absorbs `similar`'s habit of reporting an insert at the start of
   the range it just deleted.
5. **Time limits never shape output.** A wall-clock limit only fails the call with `limit_exceeded`.
   Patience above its size limit also fails with `limit_exceeded`; we never switch algorithms silently.
6. **Budget kept** (1 GB, ≤60 s, ≤2 GB), with memory measured as peak footprint, not RSS (mapped input
   pages inflate RSS by the file size).

Rejected:
- `imara-diff` 0.2.0, for the quadratic common-line pruning and a "minimal" mode that is not minimal.
- `similar` for everything, because it uses 5 to 8 times the memory and is slower on large inputs.
- `similar`'s deadline as the guard, because its output depends on machine speed.

## Consequences

- The default path uses 1.5 s and 0.4 GB on the 1 GB pair. That leaves room for word refinement, move
  detection and JSON output, and lets the 1 GB case run as a routine CI benchmark.
- Output stays deterministic on hostile inputs, and stays close to git's.
- We maintain a small Histogram fork until upstream has a work limit, and re-check it on every upgrade.
- Two libraries mean two upgrade tracks. Scaled-down spike cases go into the golden corpus, so any drift
  fails a test and forces an ID bump.
- Histogram can still be poor on a few often-repeated lines (twice the minimal edit count, as in git).
  Phase 1 may add a cheap quality check under a new ID.
- Follow-ups: faster interning, a Linux x86-64 re-run, the WASM limits (spike C), and reporting the
  0.2.0 pruning issue upstream.
- Revisit if upstream imara-diff ships the fixes and a work limit, or if `gix-imara-diff` stops being
  maintained.

## Implementation notes

Added in Phase 1, increment 1a.

- **Code.** `crates/diffyman-algo/src/line_diff.rs` holds the IDs, the budget and the `approximate`
  rule. The Histogram copy is `crates/diffyman-algo/src/line_diff/histogram.rs` plus
  `crates/diffyman-algo/src/line_diff/histogram/lcs.rs` and
  `crates/diffyman-algo/src/line_diff/histogram/list_pool.rs`, with upstream headers kept and
  `crates/diffyman-algo/src/line_diff/histogram/NOTICE` listing the changes.
- **Work units.** One per token added to the histogram, one per right-side position scanned, and one
  per occurrence visited plus one per step of extending a match. The counter is checked before each
  region and after each occurrence, so it can pass the budget by at most one extension. No clock is
  read.
- **Budget.** `max(HISTOGRAM_WORK_MIN, HISTOGRAM_WORK_PER_LINE × (n + m))` with
  `HISTOGRAM_WORK_MIN = 4,000,000` and `HISTOGRAM_WORK_PER_LINE = 128`, where `n` and `m` are the line
  counts after the common prefix and suffix are trimmed. Each level of anchor nesting costs about
  2 units per line, so 128 allows 64 levels, more than balanced nesting needs at the 2^31-line maximum.
  Past the budget, every region not yet diffed goes to `gix-imara-diff` Myers, the same Myers that
  Histogram already uses for very repetitive regions. The left-part recursion is an explicit stack
  visited in the same order, so deep inputs cannot overflow the call stack.
- **Warnings.** Item 3 names an `algorithm_fallback` warning, but the spec's warning table (§10) has
  no such code, so none is emitted. The budget does not set `approximate` either: Histogram never
  promises a minimal result, and the Myers part is covered by the existing edit-count rule
  (`is_approximate`, more than 500 changed lines whose key occurs on the other side). Callers can see a
  fallback through `line_diff::AlignReport`, which is not part of the document.
- **Patience limit.** An input over 256 MiB (268,435,456 bytes) on either side gives a
  `limit-exceeded` error and `status: "error"`, checked before any diffing.
- **IDs.** They stay `@1`: no version was released with `imara-diff` 0.2.0, and all 60 corpus cases
  give byte-identical output without re-blessing, in the CLI, Python and WASM.

Measurements on the development Mac (release build, line diff only, without reading and interning;
`cargo run --release -p diffyman-algo --example line_diff_work -- LEFT RIGHT --unguarded`):

| Case | Lines after trim | Work per line | Fell back | Histogram | Myers | Same as unguarded |
|---|---|---|---|---|---|---|
| 1 GB log | 19.3 M | 26.3 | no | 0.33 s | 0.18 s | yes |
| 100 MB log | 1.93 M | 18.9 | no | 0.02 s | 0.01 s | yes |
| 50 MB source | 3.62 M | 18.0 | no | 0.11 s | 0.05 s | yes |
| Low-frequency lines, pairs swapped | 400 k | 32.3 | no | 0.03 s | 0.38 s | yes |
| 5 M copies of one line | 10.0 M | 1.5 | no | 0.32 s | 0.33 s | yes |
| All lines different | 2.0 M | 1.0 | no | 0.002 s | 0.005 s | yes |
| 200 k unique lines, pairs swapped | 400 k | 128 (budget) | yes | **0.46 s** (unguarded: 33 s) | 0.38 s | no |

The 1 GB pair through the CLI (`diffyman diff --json`, release) takes 10.4 s wall time with 5.2 GB
max RSS and 5.7 GB peak footprint, and reports the same 48,485 changes (196,662 changed lines) as the
spike. Time is within budget; memory is not, because the CLI reads both files into memory and the
engine keeps a record and an owned comparison key per line. Large-file mode (memory-mapped input and
borrowed keys) followed in increment 1b, below.

Regression tests are in `crates/diffyman-algo/tests/line_diff_guard.rs`: 5 M identical lines with 10
edits, swapped pairs (budget hit, linear work growth, unguarded work quadratic), the low-frequency and
all-different cases, and random inputs checked for validity, equality with upstream Histogram below the
budget, and Myers minimality when `approximate` is not set. The full-size swapped-pairs timing check
runs in release with `cargo test --release -p diffyman-algo --test line_diff_guard -- --ignored`.

### Large-file mode

Added in Phase 1, increment 1b. The output is unchanged: every golden file passes in strict
conformance without re-blessing, CLI, Python and WASM stay byte-identical, and the spike-A outputs
below match the earlier build byte for byte (text, and JSON without `provenance`).

- **Input.** The CLI memory-maps regular files (`memmap2` 0.9.12, feature `mmap`, on by default and
  native-only; `--no-mmap`, pipes and empty files are read instead), with a read-ahead hint. Mapped
  pages stay in the file cache, outside the process footprint. Unsafe code is denied workspace-wide;
  the map is the one allowed exception.
- **Lines.** `lines::LineIndex` keeps only line start offsets (4 bytes per line under 4 GiB); content
  and terminator are derived from the text on demand. Since increment 1c the split lives in
  `diffyman-model` (`diffyman_model::lines`, re-exported by `diffyman-algo`), so the renderers split
  exactly as the engine does, and the CLI renders from the engine's own index (below).
- **Identical inputs.** Byte-identical sides are decoded once (one SHA-256, one line index) and never
  tokenized: the whole file is the common prefix.
- **Trim before interning.** `key::tokenize_core` finds the common prefix and suffix on raw bytes,
  extends them by comparison keys when an ignore option normalizes lines, and interns only the lines
  in between. The trimmed region is exactly the one `align` trims, so the alignment is the same, and
  token ids are a relabeling that the algorithms do not depend on. `key::is_approximate_core` gives the
  same answer as `is_approximate` on the full sequences: a changed line also counts when its key occurs
  only in the trimmed lines, which are read only when the core alone cannot decide.
- **Keys and interning.** No key is stored: each token remembers the first line that has it. With no
  normalizing option the key of a line is its raw bytes (equal raw bytes mean equal content and
  terminator). Otherwise lines with equal raw bytes still have equal keys, and two lines with equal
  hashes but different bytes build both keys again to compare them (increment 1c; before, each
  distinct key was copied into an arena). The table (`hashbrown` 0.15.5 `HashTable`, `foldhash` 0.1.5,
  fixed seed) stores a token and a 32-bit hash per distinct key, so it grows without reading keys again.
  Ids are assigned in first-seen order, so the hash function never affects output.
- **Threads.** On native targets, the two sides are decoded (SHA-256, UTF-8 check, line index) on two
  threads above 4 MiB per side, and line keys are built and hashed on two threads above 65,536 lines
  per side; ids are still assigned in order on one thread. WASM builds have no threads.
- **Output.** The canonical writer streams to any `io::Write` (and the document id is hashed as it is
  produced): each object is serialized once to read its keys, then once per key in sorted order,
  writing only that member. No text or JSON tree is held, whatever the document size.

Measured with `spikes/line-diff-bench/cli-bench.sh` (the real CLI, one process per run, file cache
warm, peak memory footprint from `/usr/bin/time -l`), release build on the development Mac:

| Case | Before: wall, footprint | After `--json` | After text diff |
|---|---|---|---|
| 1 GB log pair (9.65 M lines per side) | 8.5 s, 4,537 MB | 1.9 s, 536 MB | 2.3 s, 536 MB |
| 100 MB log pair | 0.52 s, 416 MB | 0.14 s, 52 MB | 0.17 s, 56 MB |
| 5 M copies of one line, 1,038 edits | 0.90 s, 561 MB | 0.48 s, 205 MB | 0.58 s, 205 MB |
| 1 GB log against itself | 7.3 s, 4,424 MB | 0.65 s, 58 MB | 0.66 s, 58 MB |

On the 1 GB pair about 0.6 s goes to hashing and indexing both sides, 0.9 s to interning, 0.3 s to
the line diff and 0.1 s to writing the 19 MB document. The remaining footprint is mostly the line
index (77 MB), the tokens (77 MB), the interning table (about 210 MB while it exists) and the
Histogram working set. With a cold file cache, the first pass reads the files from disk and the run
is I/O bound.

#### Rendered output and normalized keys (increment 1c)

Output is unchanged: no golden file or renderer snapshot was re-blessed, and the spike-A outputs
match the previous build byte for byte (JSON without `provenance`, text, unified and HTML, 1 GB log
pair; 7 more cases under 7 ignore-option sets).

- **Renderers reuse the engine's lines.** `diffyman::diff_document_with_text` also returns both
  decoded inputs (`TextInputs`: each text with its `LineIndex`, shared when the inputs are
  byte-identical). `diffyman_render::unified_computed` and `html_computed` take them with the input
  bytes as `Computed`, so a caller that has just computed the document (the CLI) is not hashed,
  validated and split again. They still check sizes, line counts, that each text fills its input, and
  the hunk structure, so a wrong claim can give wrong lines but never a panic. `unified` and `html`
  keep the full SHA-256 check for documents from anywhere else, now with the 4-byte index instead of
  a 24-byte line record.
- **Key building in one pass.** Whitespace modes write straight into the key buffer instead of a new
  `String` per line; ASCII runs are case folded as bytes (CaseFolding.txt maps only `A`–`Z` among
  ASCII), other characters through `icu_casemap`; under `change`, an ASCII line is copied and
  lowercased in bulk and only its runs of two or more spaces are closed up (found with a prebuilt
  `memchr::memmem` finder). A test pins the bytes to the straightforward definition for every mode.

| 1 GB log pair, CLI | Before | After |
|---|---|---|
| `--json` | 1.91 s, 537 MB | 2.02 s, 535 MB |
| text (default format) | 3.60 s, 1,021 MB | 2.12 s, 593 MB |
| `--format unified` | 3.48 s, 1,021 MB | 2.00 s, 583 MB |
| `--format html` | 3.30 s, 987 MB | 1.99 s, 546 MB |
| `--json -i --ignore-whitespace change` | 16.75 s, 1,489 MB | 2.46 s, 534 MB |
| text `-i --ignore-whitespace change` | 18.57 s, 1,974 MB | 2.60 s, 590 MB |

Before, case folding through the Unicode tables took about half of the `-i` run, the per-line
`String` and the copying of every distinct key (about 1 GB for this pair) most of the rest. Single
runs vary by up to 0.3 s on this machine; six interleaved `--json` runs of each build differ by
about 2% (1.98 s against 2.02 s median).

### Slider compaction

Added in Phase 1, increment 1c. This changes output, but no version has been released, so the IDs stay
`text.histogram@1`, `text.myers@1` and `text.patience@1` (as for the Histogram budget above); the change
is recorded here and in the status log.

- **Problem.** A run of changed lines whose first line equals the line below it, or whose last line
  equals the line above it, can be shown at several positions. The algorithms place such sliders
  wherever their search ends, so an inserted function often started with the closing brace or blank
  line of its neighbour. Moved blocks then lost a boundary line to an `equal` op (exact boundaries
  0.825 on the labeled corpus), and unified output differed from `git diff --histogram` on 14 of 40
  random edit pairs.
- **Decision.** After the line diff, every algorithm's alignment goes through slider compaction
  (spec §7.3.1): each run slides up and then down as far as it can, merging with runs it touches; it is
  lined up with a change on the other side when it can be, and otherwise placed by git's indent
  heuristic. The heuristic is always on, with no option: it is git's default since 2.14, the point is
  to agree with what people see in git, and an option would add a field to the document for little
  gain. `--no-indent-heuristic` output (runs at their lowest position) is not offered.
- **Independent implementation.** git's xdiff is LGPL, so no code was copied or translated.
  `crates/diffyman-algo/src/slider.rs` implements the behaviour from its public description: the git
  documentation of `--indent-heuristic`, Michael Haggerty's 2016 description of the heuristic and its
  tuned weights, and the observable output of git on test inputs. The weights are numeric facts of
  that published heuristic; the rules are written out in spec §7.3.1.
- **Where.** The engine compacts the whole-file alignment (`key::compact_core`), because a run may
  slide into the common prefix or suffix that the line diff trims. Inside the trimmed region lines
  compare by token; outside it by key (raw bytes when no option normalizes lines). The binary adapter
  and the sub-op alignment of move detection use `line_diff::align` directly and are not compacted.
- **Bounds.** Work units (one per shift or absorbed line, one per comparison plus one per 64 bytes
  compared outside the trimmed region, one per line the heuristic reads) under a budget of
  `max(1,000,000, 64 × lines)`; past it the pass stops and the rest stays as the algorithm left it. No
  clock. 200,000 copies of one line with one more inserted cost under 8 units per line.
- **Ignored blank lines.** Compaction keeps the number of changed lines on each side, but not which
  lines form one change op, and `ignore.blankLines` is applied after it (spec §7.3 step 3). A blank-only
  run that joins or lines up with a change that holds a non-blank line is then counted instead of
  `ignored` (or the other way round), so `linesAdded`, `linesRemoved` and `similarity` can change under
  that option. git's `--ignore-blank-lines` behaves the same way, and keeping the classification of the
  uncompacted alignment would bring back the arbitrary positions compaction removes, so this is kept
  and specified (spec §7.3.1 rule 4, case `text/043`).
- **Tests.** Unit tests in `slider/tests.rs` (lining up with the other side, the inserted-function
  case, merging, the budget, 300 random rounds checking a valid alignment with the same changed keys,
  linear work on repeated lines), and the trimmed-path tests in `key.rs` now compare compaction over
  the trimmed core with compaction over the full token sequences for every whitespace mode, case
  folding and regex masks.

Results (release build, git 2.54):

| Check | Before | After |
|---|---|---|
| Exact boundaries, 40 matched labeled moves | 0.825 (33) | 1.000 (40) |
| 40 random edit pairs of repository files, unified output equal to `git diff --histogram` | 26 | 40 |
| 100 more random pairs, same check | 77 | 100 |
| The same 100 pairs, `--algorithm myers` against `git diff --diff-algorithm=myers` | not measured | 100 |
| Spike-A 50 MB source pair, unified output equal to git's | no (1,563 of 2,560 hunk headers) | yes, byte for byte |
| Text corpus, `compare_unified.py` (git without, then with, the indent heuristic) | 25 of 36 | 25 of 36 |

Re-blessing changed 15 golden files (`text/037` and `moved/` 001, 002, 004, 007, 008, 010, 011, 012,
013, 014, 026, 032, 041, 046) and 6 renderer snapshot files. In every one the changed-line counts,
`linesUnchanged` and `similarity` are the same; only positions moved (an inserted or deleted function
now starts at its header and takes its own blank line or brace), `hunkCount` changed in two cases
(`moved/032`, `moved/041`: one hunk split in two) and seven moves gained the boundary line their label
always had. The remaining text-corpus differences from git are VT and FF under `-b`, git's
`--ignore-blank-lines` grouping, lone CR lines and where context lines come from under an ignore
option (renderer README). Timing and memory through the CLI are unchanged within noise: the 1 GB log
pair takes 1.95 s and 537 MB with `--json` (was 1.86 s and 537 MB), the 50 MB source pair 0.27 s (was
0.25 s), 5 M copies of one line 0.55 s (was 0.54 s). The 1 GB pair has 5 fewer changes and 6 fewer
hunks, with the same 98,992 added and 97,670 removed lines.
