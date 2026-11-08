//! Line diff behind versioned algorithm IDs (Diff Document spec §7.3, §12).
//!
//! The result is an [`Alignment`]: which left lines are removed and which
//! right lines are added. The remaining lines match up in order.
//!
//! Each algorithm ID is defined by this code plus its pinned library:
//!
//! | ID | Library |
//! |---|---|
//! | `text.histogram@1` | `imara-diff` 0.2.0, `Algorithm::Histogram` |
//! | `text.myers@1` | `imara-diff` 0.2.0, `Algorithm::Myers` (with its heuristics) |
//! | `text.patience@1` | `similar` 3.2.1, `Algorithm::Patience` |
//!
//! The common prefix and suffix of the two token sequences are trimmed before
//! the library runs. No slider or indent post-processing is applied.
//!
//! Both libraries bound their Myers search with a cost limit (Myers directly,
//! histogram through its Myers fallback, patience between its anchors). The
//! limit only engages after 256 search rounds on a sub-problem, and those
//! rounds find every edit script of up to 511 edits. A sub-problem's minimal
//! edit count is at most the number of changed lines whose key also occurs on
//! the other side. [`is_approximate`] flags a result where that count is over
//! [`EXACT_MAX_EDITS`]: below it the limit provably did not engage. Above it
//! the result may or may not be minimal, so the flag is conservative.

use std::convert::Infallible;

use imara_diff::{Algorithm as ImaraAlgorithm, Diff, Token};
use similar::algorithms::DiffHook;

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
    assert!(
        left.len() <= MAX_LINES && right.len() <= MAX_LINES,
        "too many lines"
    );
    let mut alignment = Alignment {
        removed: vec![false; left.len()],
        added: vec![false; right.len()],
    };
    let prefix = common_prefix(left, right);
    let suffix = common_suffix(&left[prefix..], &right[prefix..]);
    let l = &left[prefix..left.len() - suffix];
    let r = &right[prefix..right.len() - suffix];
    if l.is_empty() || r.is_empty() {
        alignment.removed[prefix..prefix + l.len()].fill(true);
        alignment.added[prefix..prefix + r.len()].fill(true);
        return alignment;
    }
    match algorithm {
        LineAlgorithm::Histogram | LineAlgorithm::Myers => {
            let imara = if algorithm == LineAlgorithm::Histogram {
                ImaraAlgorithm::Histogram
            } else {
                ImaraAlgorithm::Myers
            };
            let lt: Vec<Token> = l.iter().copied().map(Token::from).collect();
            let rt: Vec<Token> = r.iter().copied().map(Token::from).collect();
            let mut diff = Diff::default();
            diff.compute_with(imara, &lt, &rt, num_tokens);
            for i in 0..l.len() {
                alignment.removed[prefix + i] = diff.is_removed(index_u32(i));
            }
            for j in 0..r.len() {
                alignment.added[prefix + j] = diff.is_added(index_u32(j));
            }
        }
        LineAlgorithm::Patience => {
            let mut hook = FlagHook {
                removed: &mut alignment.removed[prefix..prefix + l.len()],
                added: &mut alignment.added[prefix..prefix + r.len()],
            };
            let Ok(()) =
                similar::algorithms::diff_slices(similar::Algorithm::Patience, &mut hook, l, r);
        }
    }
    alignment
}

/// Most changed lines (whose key also occurs on the other side) for which the
/// line diff is known to be free of the libraries' cost limits.
pub const EXACT_MAX_EDITS: usize = 500;

/// True when a cost limit may have cut the search for `alignment` short (the
/// Diff Document's `approximate` warning). `left` and `right` are the token
/// sequences passed to [`align`].
pub fn is_approximate(left: &[u32], right: &[u32], alignment: &Alignment) -> bool {
    let presence =
        |side: &[u32]| -> std::collections::HashSet<u32> { side.iter().copied().collect() };
    let (in_left, in_right) = (presence(left), presence(right));
    let removed = left
        .iter()
        .zip(&alignment.removed)
        .filter(|(t, r)| **r && in_right.contains(t))
        .count();
    let added = right
        .iter()
        .zip(&alignment.added)
        .filter(|(t, a)| **a && in_left.contains(t))
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

    /// Exact (`MyersMinimal`) imara-diff result on the same trimmed input.
    fn minimal(l: &[u32], r: &[u32], n: u32) -> Alignment {
        let prefix = common_prefix(l, r);
        let suffix = common_suffix(&l[prefix..], &r[prefix..]);
        let (lm, rm) = (&l[prefix..l.len() - suffix], &r[prefix..r.len() - suffix]);
        let lt: Vec<Token> = lm.iter().copied().map(Token::from).collect();
        let rt: Vec<Token> = rm.iter().copied().map(Token::from).collect();
        let mut diff = Diff::default();
        diff.compute_with(ImaraAlgorithm::MyersMinimal, &lt, &rt, n);
        let mut a = Alignment {
            removed: vec![false; l.len()],
            added: vec![false; r.len()],
        };
        for i in 0..lm.len() {
            a.removed[prefix + i] = diff.is_removed(index_u32(i));
        }
        for j in 0..rm.len() {
            a.added[prefix + j] = diff.is_added(index_u32(j));
        }
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
                assert_eq!(a, minimal(&l, &r, alphabet), "round {round}");
                exact_checked += 1;
            }
        }
        assert!(exact_checked > 10);
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
