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
borrowed keys) is the next increment.

Regression tests are in `crates/diffyman-algo/tests/line_diff_guard.rs`: 5 M identical lines with 10
edits, swapped pairs (budget hit, linear work growth, unguarded work quadratic), the low-frequency and
all-different cases, and random inputs checked for validity, equality with upstream Histogram below the
budget, and Myers minimality when `approximate` is not set. The full-size swapped-pairs timing check
runs in release with `cargo test --release -p diffyman-algo --test line_diff_guard -- --ignored`.
