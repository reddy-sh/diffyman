//! Line diff behind versioned algorithm IDs (Diff Document spec §7.3, §12;
//! ADR 0006).
//!
//! The result is an [`Alignment`]: which left lines are removed and which
//! right lines are added. The remaining lines match up in order.
//!
//! Each algorithm ID is defined by this code plus its pinned library:
//!
//! | ID | Engine |
//! |---|---|
//! | `text.histogram@1` | Histogram copied from `gix-imara-diff` 0.3.3 with a work budget ([`HISTOGRAM_WORK_PER_LINE`]), falling back to its Myers |
//! | `text.myers@1` | `gix-imara-diff` 0.3.3, `Algorithm::Myers` (with its heuristics) |
//! | `text.patience@1` | `similar` 3.2.1, `Algorithm::Patience`, inputs up to [`PATIENCE_MAX_INPUT_BYTES`] per side |
//!
//! The common prefix and suffix of the two token sequences are trimmed before
//! the algorithm runs. No slider or indent post-processing is applied.
//!
//! # Histogram work budget
//!
//! Histogram is quadratic on some inputs (for example, unique lines with every
//! adjacent pair swapped: each step finds a one-line anchor at the edge of the
//! region and rebuilds the histogram for almost all of it). The copy in
//! `histogram.rs` counts its work in abstract units: one per token added to the
//! histogram, one per scanned right-side position, and one per occurrence
//! visited plus one per step of extending a match. No clock is involved, so the
//! count is the same on every platform and binding. The budget is
//! `max(HISTOGRAM_WORK_MIN, HISTOGRAM_WORK_PER_LINE * (n + m))`, where `n` and
//! `m` are the line counts left after trimming. Once the counter is past the
//! budget, every region not yet diffed is diffed with Myers instead (the same
//! Myers that Histogram already uses for very repetitive regions). Below the
//! budget the output is identical to unguarded `gix-imara-diff` Histogram.
//!
//! # The `approximate` warning
//!
//! [`is_approximate`] reports whether a *cost limit inside Myers* may have cut
//! a search short, which is the one way these algorithms can return a result
//! that is not minimal by their own definition. Both libraries bound their
//! Myers search (Myers directly, Histogram through its Myers fallback,
//! Patience between its anchors). The limit only engages after 256 search
//! rounds on a sub-problem, and those rounds find every edit script of up to
//! 511 edits. A sub-problem's minimal edit count is at most the number of
//! changed lines whose key also occurs on the other side. [`is_approximate`]
//! flags a result where that count is over [`EXACT_MAX_EDITS`]: below it the
//! limit provably did not engage. Above it the result may or may not be
//! minimal, so the flag is conservative.
//!
//! The Histogram budget does not set the warning by itself. Histogram never
//! promises a minimal result, and past the budget the remaining regions get
//! Myers, whose own cost limit the same edit-count rule covers. So the rule
//! above stays exact for all three IDs. (The swapped-pairs input above is
//! flagged anyway, because its changed lines all occur on the other side.)

use std::convert::Infallible;

use gix_imara_diff::{Algorithm as ImaraAlgorithm, Diff, Token};
use similar::algorithms::DiffHook;

mod histogram;

/// A line diff algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineAlgorithm {
    /// `text.histogram@1`.
    #[default]
    Histogram,
    /// `text.myers@1`.
    Myers,
    /// `text.patience@1`.
    Patience,
}

impl LineAlgorithm {
    /// The versioned algorithm ID.
    pub fn id(self) -> &'static str {
        match self {
            LineAlgorithm::Histogram => "text.histogram@1",
            LineAlgorithm::Myers => "text.myers@1",
            LineAlgorithm::Patience => "text.patience@1",
        }
    }
}

/// Largest number of lines per side the line diff accepts.
pub const MAX_LINES: usize = (i32::MAX - 1) as usize;

/// Largest input per side, in bytes, that `text.patience@1` accepts (256 MiB).
/// `similar` uses several times the input size in memory, so larger inputs
/// fail with `limit-exceeded` instead of switching algorithms (ADR 0006).
pub const PATIENCE_MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;

/// Histogram work units allowed per line left after trimming (both sides
/// counted). Each level of anchor nesting costs about 2 units per line, so 128
/// allows 64 levels: more than balanced nesting needs even at [`MAX_LINES`]
/// (31 levels). The spike corpus peaks at 26 units per line on a 1 GB log pair
/// and 32 on its low-frequency case (ADR 0006, implementation notes).
pub const HISTOGRAM_WORK_PER_LINE: u64 = 128;

/// Smallest Histogram budget, so small inputs never fall back.
pub const HISTOGRAM_WORK_MIN: u64 = 4_000_000;

/// The Histogram work budget for trimmed inputs of `left_lines` and
/// `right_lines` lines.
pub fn histogram_budget(left_lines: usize, right_lines: usize) -> u64 {
    let lines = (left_lines as u64).saturating_add(right_lines as u64);
    HISTOGRAM_WORK_PER_LINE
        .saturating_mul(lines)
        .max(HISTOGRAM_WORK_MIN)
}

/// True when inputs of these sizes (bytes per side) are within `algorithm`'s
/// size limit. Only `text.patience@1` has one ([`PATIENCE_MAX_INPUT_BYTES`]).
pub fn within_size_limit(algorithm: LineAlgorithm, left_bytes: u64, right_bytes: u64) -> bool {
    match algorithm {
        LineAlgorithm::Patience => {
            left_bytes <= PATIENCE_MAX_INPUT_BYTES && right_bytes <= PATIENCE_MAX_INPUT_BYTES
        }
        LineAlgorithm::Histogram | LineAlgorithm::Myers => true,
    }
}

/// Matched and unmatched lines of a diff.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Alignment {
    /// `removed[i]`: left line `i` has no match.
    pub removed: Vec<bool>,
    /// `added[j]`: right line `j` has no match.
    pub added: Vec<bool>,
}

impl Alignment {
    /// True when the matched lines on both sides are equal in number.
    pub fn is_consistent(&self) -> bool {
        self.removed.iter().filter(|r| !**r).count() == self.added.iter().filter(|a| !**a).count()
    }
}

/// Audit data about one [`align_with_report`] run. Deterministic, but not part
/// of the Diff Document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AlignReport {
    /// Histogram work units spent (0 for other algorithms).
    pub histogram_work: u64,
    /// The Histogram budget that applied (0 for other algorithms).
    pub histogram_budget: u64,
    /// True when the budget ran out and the rest was diffed with Myers.
    pub histogram_fell_back: bool,
}

fn common_prefix(a: &[u32], b: &[u32]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn common_suffix(a: &[u32], b: &[u32]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

/// Runs `algorithm` over two token sequences. `num_tokens` is one more than the
/// largest token ID (the interner size).
///
/// # Panics
///
/// If either side has more than [`MAX_LINES`] tokens.
pub fn align(algorithm: LineAlgorithm, left: &[u32], right: &[u32], num_tokens: u32) -> Alignment {
    align_with_report(algorithm, left, right, num_tokens).0
}

/// Like [`align`], and also returns what the Histogram budget did.
pub fn align_with_report(
    algorithm: LineAlgorithm,
    left: &[u32],
    right: &[u32],
    num_tokens: u32,
) -> (Alignment, AlignReport) {
    align_inner(algorithm, left, right, num_tokens, None)
}

/// `text.histogram@1` with an explicit work budget instead of
/// [`histogram_budget`]. For tests and benchmarks: a different budget gives a
/// result outside any algorithm ID. `u64::MAX` gives unguarded Histogram.
pub fn align_histogram_with_budget(
    left: &[u32],
    right: &[u32],
    num_tokens: u32,
    budget: u64,
) -> (Alignment, AlignReport) {
    align_inner(
        LineAlgorithm::Histogram,
        left,
        right,
        num_tokens,
        Some(budget),
    )
}

fn align_inner(
    algorithm: LineAlgorithm,
    left: &[u32],
    right: &[u32],
    num_tokens: u32,
    budget: Option<u64>,
) -> (Alignment, AlignReport) {
    assert!(
        left.len() <= MAX_LINES && right.len() <= MAX_LINES,
        "too many lines"
    );
    let mut alignment = Alignment {
        removed: vec![false; left.len()],
        added: vec![false; right.len()],
    };
    let mut report = AlignReport::default();
    let prefix = common_prefix(left, right);
    let suffix = common_suffix(&left[prefix..], &right[prefix..]);
    let l = &left[prefix..left.len() - suffix];
    let r = &right[prefix..right.len() - suffix];
    let removed = &mut alignment.removed[prefix..prefix + l.len()];
    let added = &mut alignment.added[prefix..prefix + r.len()];
    if l.is_empty() || r.is_empty() {
        removed.fill(true);
        added.fill(true);
        return (alignment, report);
    }
    match algorithm {
        LineAlgorithm::Histogram => {
            let budget = budget.unwrap_or_else(|| histogram_budget(l.len(), r.len()));
            let lt = tokens(l);
            let rt = tokens(r);
            let outcome = histogram::diff(&lt, &rt, removed, added, num_tokens, budget);
            report = AlignReport {
                histogram_work: outcome.work,
                histogram_budget: budget,
                histogram_fell_back: outcome.fell_back,
            };
        }
        LineAlgorithm::Myers => {
            let lt = tokens(l);
            let rt = tokens(r);
            let mut diff = Diff::default();
            diff.compute_with(ImaraAlgorithm::Myers, &lt, &rt, num_tokens);
            copy_flags(&diff, removed, added);
        }
        LineAlgorithm::Patience => {
            let mut hook = FlagHook { removed, added };
            let Ok(()) =
                similar::algorithms::diff_slices(similar::Algorithm::Patience, &mut hook, l, r);
        }
    }
    (alignment, report)
}

fn tokens(side: &[u32]) -> Vec<Token> {
    side.iter().copied().map(Token::from).collect()
}

fn copy_flags(diff: &Diff, removed: &mut [bool], added: &mut [bool]) {
    for (i, flag) in removed.iter_mut().enumerate() {
        *flag = diff.is_removed(index_u32(i));
    }
    for (j, flag) in added.iter_mut().enumerate() {
        *flag = diff.is_added(index_u32(j));
    }
}

/// Most changed lines (whose key also occurs on the other side) for which the
/// line diff is known to be free of the libraries' cost limits.
pub const EXACT_MAX_EDITS: usize = 500;

/// True when a cost limit may have cut the search for `alignment` short (the
/// Diff Document's `approximate` warning). `left` and `right` are the token
/// sequences passed to [`align`]. See the module docs for the rule.
pub fn is_approximate(left: &[u32], right: &[u32], alignment: &Alignment) -> bool {
    let size = left
        .iter()
        .chain(right)
        .copied()
        .max()
        .map_or(0, |m| m as usize + 1);
    let presence = |side: &[u32]| -> Vec<bool> {
        let mut seen = vec![false; size];
        for &t in side {
            seen[t as usize] = true;
        }
        seen
    };
    let (in_left, in_right) = (presence(left), presence(right));
    let removed = left
        .iter()
        .zip(&alignment.removed)
        .filter(|(t, r)| **r && in_right[**t as usize])
        .count();
    let added = right
        .iter()
        .zip(&alignment.added)
        .filter(|(t, a)| **a && in_left[**t as usize])
        .count();
    removed + added > EXACT_MAX_EDITS
}

fn index_u32(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

/// Runs the token-level Myers diff used for inline refinement (`similar` 3.2.1).
pub fn align_tokens_myers(left: &[u32], right: &[u32]) -> Alignment {
    let mut alignment = Alignment {
        removed: vec![false; left.len()],
        added: vec![false; right.len()],
    };
    let mut hook = FlagHook {
        removed: &mut alignment.removed,
        added: &mut alignment.added,
    };
    let Ok(()) =
        similar::algorithms::diff_slices(similar::Algorithm::Myers, &mut hook, left, right);
    alignment
}

/// A `similar` hook that records deleted and inserted indices as flags.
struct FlagHook<'a> {
    removed: &'a mut [bool],
    added: &'a mut [bool],
}

impl DiffHook for FlagHook<'_> {
    type Error = Infallible;

    fn delete(
        &mut self,
        old_index: usize,
        old_len: usize,
        _new_index: usize,
    ) -> Result<(), Infallible> {
        self.removed[old_index..old_index + old_len].fill(true);
        Ok(())
    }

    fn insert(
        &mut self,
        _old_index: usize,
        new_index: usize,
        new_len: usize,
    ) -> Result<(), Infallible> {
        self.added[new_index..new_index + new_len].fill(true);
        Ok(())
    }

    fn replace(
        &mut self,
        old_index: usize,
        old_len: usize,
        new_index: usize,
        new_len: usize,
    ) -> Result<(), Infallible> {
        self.removed[old_index..old_index + old_len].fill(true);
        self.added[new_index..new_index + new_len].fill(true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [LineAlgorithm; 3] = [
        LineAlgorithm::Histogram,
        LineAlgorithm::Myers,
        LineAlgorithm::Patience,
    ];

    fn run(alg: LineAlgorithm, l: &[u32], r: &[u32]) -> Alignment {
        let n = l.iter().chain(r).copied().max().map_or(0, |m| m + 1);
        align(alg, l, r, n)
    }

    #[test]
    fn identical_and_empty() {
        for alg in ALL {
            let a = run(alg, &[1, 2, 3], &[1, 2, 3]);
            assert!(a.removed.iter().all(|x| !x) && a.added.iter().all(|x| !x));
            let a = run(alg, &[], &[1, 2]);
            assert_eq!(a.added, [true, true]);
            let a = run(alg, &[1, 2], &[]);
            assert_eq!(a.removed, [true, true]);
        }
    }

    #[test]
    fn single_replace() {
        for alg in ALL {
            let a = run(alg, &[0, 1, 2, 3, 4], &[0, 5, 2, 3, 4, 6]);
            assert_eq!(a.removed, [false, true, false, false, false], "{alg:?}");
            assert_eq!(a.added, [false, true, false, false, false, true], "{alg:?}");
            assert!(a.is_consistent());
        }
    }

    #[test]
    fn alignments_are_consistent() {
        let l = [1, 2, 3, 1, 2, 3, 4, 5, 1, 9, 9, 2];
        let r = [3, 1, 2, 4, 4, 5, 9, 1, 2, 3, 7];
        for alg in ALL {
            assert!(run(alg, &l, &r).is_consistent(), "{alg:?}");
        }
    }

    #[test]
    fn ids() {
        assert_eq!(LineAlgorithm::Histogram.id(), "text.histogram@1");
        assert_eq!(LineAlgorithm::Myers.id(), "text.myers@1");
        assert_eq!(LineAlgorithm::Patience.id(), "text.patience@1");
    }

    #[test]
    fn budget_is_size_proportional_with_a_floor() {
        assert_eq!(histogram_budget(0, 0), HISTOGRAM_WORK_MIN);
        assert_eq!(histogram_budget(10, 10), HISTOGRAM_WORK_MIN);
        assert_eq!(
            histogram_budget(1_000_000, 3_000_000),
            HISTOGRAM_WORK_PER_LINE * 4_000_000
        );
        assert_eq!(histogram_budget(usize::MAX, usize::MAX), u64::MAX);
    }

    #[test]
    fn patience_size_limit() {
        let max = PATIENCE_MAX_INPUT_BYTES;
        assert!(within_size_limit(LineAlgorithm::Patience, max, max));
        assert!(!within_size_limit(LineAlgorithm::Patience, max + 1, 0));
        assert!(!within_size_limit(LineAlgorithm::Patience, 0, max + 1));
        assert!(within_size_limit(
            LineAlgorithm::Histogram,
            u64::MAX,
            u64::MAX
        ));
        assert!(within_size_limit(LineAlgorithm::Myers, u64::MAX, u64::MAX));
    }

    /// Small deterministic generator (xorshift64*), so the tests need no crate.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self, bound: u32) -> u32 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let x = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32;
            u32::try_from(x % u64::from(bound)).unwrap()
        }
    }

    /// `left` of `n` tokens over `alphabet`, and a copy with `edits` random
    /// single-token deletes, inserts and substitutions.
    fn edited(rng: &mut Rng, n: usize, alphabet: u32, edits: usize) -> (Vec<u32>, Vec<u32>) {
        let left: Vec<u32> = (0..n).map(|_| rng.next(alphabet)).collect();
        let mut right = left.clone();
        for _ in 0..edits {
            let at = rng.next(u32::try_from(right.len()).unwrap() + 1) as usize;
            match rng.next(3) {
                0 if at < right.len() => {
                    right.remove(at);
                }
                1 if at < right.len() => right[at] = rng.next(alphabet),
                _ => right.insert(at, rng.next(alphabet)),
            }
        }
        (left, right)
    }

    /// `gix-imara-diff` with `algorithm` on the same trimmed input.
    fn reference(algorithm: ImaraAlgorithm, l: &[u32], r: &[u32], n: u32) -> Alignment {
        let prefix = common_prefix(l, r);
        let suffix = common_suffix(&l[prefix..], &r[prefix..]);
        let (lm, rm) = (&l[prefix..l.len() - suffix], &r[prefix..r.len() - suffix]);
        let mut diff = Diff::default();
        diff.compute_with(algorithm, &tokens(lm), &tokens(rm), n);
        let mut a = Alignment {
            removed: vec![false; l.len()],
            added: vec![false; r.len()],
        };
        copy_flags(
            &diff,
            &mut a.removed[prefix..prefix + lm.len()],
            &mut a.added[prefix..prefix + rm.len()],
        );
        a
    }

    #[test]
    fn not_approximate_means_exact() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut exact_checked = 0;
        for round in 0..60 {
            let n = 200 + 40 * (round % 10);
            let alphabet = [8, 40, 400][round % 3];
            let edits = 20 + 12 * round;
            let (l, r) = edited(&mut rng, n, alphabet, edits);
            let a = run(LineAlgorithm::Myers, &l, &r);
            if !is_approximate(&l, &r, &a) {
                assert_eq!(
                    a,
                    reference(ImaraAlgorithm::MyersMinimal, &l, &r, alphabet),
                    "round {round}"
                );
                exact_checked += 1;
            }
        }
        assert!(exact_checked > 10);
    }

    #[test]
    fn histogram_copy_matches_upstream_below_budget() {
        let mut rng = Rng(0xD1FF_7A11);
        for round in 0..80 {
            let n = 50 + 37 * (round % 20);
            let alphabet = [3, 16, 90, 2000][round % 4];
            let edits = 1 + 7 * (round % 13);
            let (l, r) = edited(&mut rng, n, alphabet, edits);
            let (a, report) = align_with_report(LineAlgorithm::Histogram, &l, &r, alphabet);
            assert!(!report.histogram_fell_back, "round {round}");
            assert_eq!(
                a,
                reference(ImaraAlgorithm::Histogram, &l, &r, alphabet),
                "round {round}"
            );
        }
    }

    #[test]
    fn zero_budget_is_myers() {
        let mut rng = Rng(42);
        for round in 0..40 {
            let alphabet = [5, 60, 600][round % 3];
            let (l, r) = edited(&mut rng, 300, alphabet, 10 + 5 * round);
            let (a, report) = align_histogram_with_budget(&l, &r, alphabet, 0);
            if a.removed.iter().chain(&a.added).any(|x| *x) {
                assert!(report.histogram_fell_back, "round {round}");
            }
            assert_eq!(a, run(LineAlgorithm::Myers, &l, &r), "round {round}");
            if !is_approximate(&l, &r, &a) {
                assert_eq!(
                    a,
                    reference(ImaraAlgorithm::MyersMinimal, &l, &r, alphabet),
                    "round {round}"
                );
            }
        }
    }

    #[test]
    fn large_unrelated_inputs_are_approximate() {
        let mut rng = Rng(7);
        let l: Vec<u32> = (0..3000).map(|_| rng.next(50)).collect();
        let r: Vec<u32> = (0..3000).map(|_| rng.next(50)).collect();
        for alg in ALL {
            assert!(is_approximate(&l, &r, &run(alg, &l, &r)), "{alg:?}");
        }
        // A small edit of a large file stays exact.
        let (l, r) = edited(&mut rng, 3000, 50, 30);
        for alg in ALL {
            assert!(!is_approximate(&l, &r, &run(alg, &l, &r)), "{alg:?}");
        }
        // Lines unique to one side never count.
        let l: Vec<u32> = (0..2000).collect();
        let r: Vec<u32> = (2000..4000).collect();
        assert!(!is_approximate(&l, &r, &run(LineAlgorithm::Myers, &l, &r)));
    }

    #[test]
    fn token_myers() {
        let a = align_tokens_myers(&[1, 2, 3], &[1, 4, 3]);
        assert_eq!(a.removed, [false, true, false]);
        assert_eq!(a.added, [false, true, false]);
    }
}
