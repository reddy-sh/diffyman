// SPDX-License-Identifier: Apache-2.0
//
// Copied from gix-imara-diff 0.3.3 (src/histogram.rs), Apache-2.0.
// Changes for diffyman (see NOTICE in ./histogram/):
// - A deterministic work counter with a budget. Once the counter is past the
//   budget, every region still to be diffed goes to Myers instead.
// - The recursion on the left part is an explicit stack, processed in the
//   same order, so deep inputs cannot overflow the call stack.
// - Myers is reached through the public `Diff::compute_with` API, which runs
//   the same `myers::diff(.., minimal = false)` as the upstream fallback.
// Without a budget hit the output is identical to upstream Histogram.
//
// Original header:
// Modified for gitoxide from the upstream imara-diff crate.
// Upstream source: git cat-file -p 32d1e45d3df061e6ccba6db7fdce92db29e345d8:src/histogram.rs

// Kept close to upstream so upgrades are easy to re-check; pedantic lints are
// not applied to this copy.
#![allow(clippy::pedantic)]

use std::ops::Range;

use gix_imara_diff::{Algorithm, Diff, Token};

use lcs::find_lcs;
use list_pool::{ListHandle, ListPool};

mod lcs;
mod list_pool;

/// Maximum number of occurrences tracked for a single token.
/// Tokens appearing more frequently fall back to Myers algorithm.
const MAX_CHAIN_LEN: u32 = 63;

/// What the guarded Histogram run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Outcome {
    /// Work units spent (see `line_diff::HISTOGRAM_WORK_PER_LINE`).
    pub work: u64,
    /// True when the budget ran out and the rest was diffed with Myers.
    pub fell_back: bool,
}

/// State for computing histogram-based diffs.
struct Histogram {
    /// Tracks where each token appears in the "before" sequence.
    token_occurrences: Vec<ListHandle>,
    /// Memory pool for efficiently storing occurrence lists.
    pool: ListPool,
    /// diffyman: work units spent so far.
    work: u64,
    /// diffyman: the budget; past it, regions go to Myers.
    limit: u64,
    /// diffyman: reused Myers output buffers.
    myers: Diff,
    num_tokens: u32,
}

/// Computes a diff using the histogram algorithm, with a work budget.
///
/// # Parameters
///
/// * `before` - The token sequence from the first file, before changes.
/// * `after` - The token sequence from the second file, after changes.
/// * `removed` - Output array marking removed tokens (all false on entry)
/// * `added` - Output array marking added tokens (all false on entry)
/// * `num_tokens` - The total number of distinct tokens
/// * `limit` - The work budget
pub(crate) fn diff(
    before: &[Token],
    after: &[Token],
    removed: &mut [bool],
    added: &mut [bool],
    num_tokens: u32,
    limit: u64,
) -> Outcome {
    let mut histogram = Histogram::new(num_tokens, limit);
    let fell_back = histogram.run(before, after, removed, added);
    Outcome {
        work: histogram.work,
        fell_back,
    }
}

impl Histogram {
    fn new(num_buckets: u32, limit: u64) -> Histogram {
        Histogram {
            token_occurrences: vec![ListHandle::default(); num_buckets as usize],
            pool: ListPool::new(2 * num_buckets),
            work: 0,
            limit,
            myers: Diff::default(),
            num_tokens: num_buckets,
        }
    }

    fn clear(&mut self) {
        self.pool.clear();
    }

    fn token_occurrences(&self, token: Token) -> &[u32] {
        self.token_occurrences[token.0 as usize].as_slice(&self.pool)
    }

    fn num_token_occurrences(&self, token: Token) -> u32 {
        self.token_occurrences[token.0 as usize].len(&self.pool)
    }

    fn populate(&mut self, file: &[Token]) {
        self.charge(file.len() as u64);
        for (i, &token) in file.iter().enumerate() {
            self.token_occurrences[token.0 as usize].push(i as u32, &mut self.pool);
        }
    }

    /// diffyman: adds `units` to the work counter.
    fn charge(&mut self, units: u64) {
        self.work = self.work.saturating_add(units);
    }

    /// diffyman: true once the work counter is past the budget.
    fn over_budget(&self) -> bool {
        self.work > self.limit
    }

    /// diffyman: true if charging `units` more would pass the budget.
    fn would_exceed(&self, units: u64) -> bool {
        self.work.saturating_add(units) > self.limit
    }

    /// Myers with heuristics on one region, as in the upstream fallback.
    fn myers(
        &mut self,
        before: &[Token],
        after: &[Token],
        removed: &mut [bool],
        added: &mut [bool],
    ) {
        self.myers
            .compute_with(Algorithm::Myers, before, after, self.num_tokens);
        for (i, flag) in removed.iter_mut().enumerate() {
            *flag = self.myers.is_removed(i as u32);
        }
        for (j, flag) in added.iter_mut().enumerate() {
            *flag = self.myers.is_added(j as u32);
        }
    }

    /// Diffs every region, left to right. Returns true if the budget ran out.
    fn run(
        &mut self,
        before: &[Token],
        after: &[Token],
        removed: &mut [bool],
        added: &mut [bool],
    ) -> bool {
        let mut fell_back = false;
        // Regions still to do. Upstream recurses into the part left of the LCS
        // and then loops on the part right of it; pushing the right part first
        // keeps that order.
        let mut stack: Vec<(Range<usize>, Range<usize>)> = vec![(0..before.len(), 0..after.len())];
        while let Some((b, a)) = stack.pop() {
            let (region_before, region_after) = (&before[b.clone()], &after[a.clone()]);
            if region_before.is_empty() {
                added[a].fill(true);
                continue;
            } else if region_after.is_empty() {
                removed[b].fill(true);
                continue;
            }

            if self.over_budget() {
                fell_back = true;
                self.myers(region_before, region_after, &mut removed[b], &mut added[a]);
                continue;
            }

            self.populate(region_before);
            let lcs = find_lcs(region_before, region_after, self);
            if self.over_budget() {
                fell_back = true;
                self.myers(region_before, region_after, &mut removed[b], &mut added[a]);
                continue;
            }
            match lcs {
                // no lcs was found, that means that file1 and file2 two have nothing in common
                Some(lcs) if lcs.len == 0 => {
                    added[a].fill(true);
                    removed[b].fill(true);
                }
                Some(lcs) => {
                    let before_end = (lcs.before_start + lcs.len) as usize;
                    let after_end = (lcs.after_start + lcs.len) as usize;
                    stack.push((b.start + before_end..b.end, a.start + after_end..a.end));
                    stack.push((
                        b.start..b.start + lcs.before_start as usize,
                        a.start..a.start + lcs.after_start as usize,
                    ));
                }
                None => {
                    // we are diffing two extremely large repetitive files
                    // this is a worst case for histogram diff with O(N^2) performance
                    // fallback to myers to maintain linear time complexity
                    self.myers(region_before, region_after, &mut removed[b], &mut added[a]);
                }
            }
        }
        fell_back
    }
}
