# 0006. Line diff library and algorithm IDs

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

Every text, code and document diff ends in a line diff, so this choice sets most of diffyman's speed,
memory use and output quality. The plan names two Apache-2.0 crates, `similar` and `imara-diff`, and sets
a Phase 1 budget: a 1 GB log pair in ≤60 s with ≤2 GB peak memory on the development Mac. The output must
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
