//! Regression tests for the line diff on large and pathological inputs
//! (ADR 0006): the `text.histogram@1` work budget, the switch away from
//! `imara-diff` 0.2.0, and output validity.
//!
//! The default tests run in a few seconds in a debug build; their wall-time
//! bounds are loose and only catch hangs. The `#[ignore]`d test uses the full
//! swapped-pairs size and a tight time bound, so run it in release:
//!
//! ```text
//! cargo test --release -p diffyman-algo --test line_diff_guard -- --ignored
//! ```

use std::time::{Duration, Instant};

use diffyman_algo::line_diff::{
    self, AlignReport, Alignment, LineAlgorithm, align, align_histogram_with_budget,
    align_with_report, histogram_budget, is_approximate,
};
use diffyman_algo::ops::{OpKind, full_ops};
use gix_imara_diff::{Algorithm, Diff, Token};

const ALL: [LineAlgorithm; 3] = [
    LineAlgorithm::Histogram,
    LineAlgorithm::Myers,
    LineAlgorithm::Patience,
];

/// Small deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32;
        usize::try_from(x % bound as u64).unwrap()
    }
}

fn num_tokens(l: &[u32], r: &[u32]) -> u32 {
    l.iter().chain(r).copied().max().map_or(0, |m| m + 1)
}

/// Applies the ops to `left` and checks the result is `right`: every `equal`
/// op pairs equal lines, and the ops tile both sides.
fn assert_valid(left: &[u32], right: &[u32], a: &Alignment) {
    assert_eq!(a.removed.len(), left.len());
    assert_eq!(a.added.len(), right.len());
    let mut rebuilt = Vec::with_capacity(right.len());
    let (mut i, mut j) = (0, 0);
    for op in full_ops(a) {
        assert_eq!((op.left_start, op.right_start), (i, j), "ops must tile");
        if op.kind == OpKind::Equal {
            assert_eq!(op.left_len, op.right_len);
            rebuilt.extend_from_slice(&left[i..i + op.left_len]);
        } else {
            rebuilt.extend_from_slice(&right[j..j + op.right_len]);
        }
        i += op.left_len;
        j += op.right_len;
    }
    assert_eq!((i, j), (left.len(), right.len()));
    assert!(
        rebuilt == right,
        "applying the ops must give the right side"
    );
}

fn changed(a: &Alignment) -> usize {
    a.removed.iter().chain(&a.added).filter(|x| **x).count()
}

/// Unguarded `gix-imara-diff` with `algorithm`, on the same trimmed input.
fn upstream(algorithm: Algorithm, left: &[u32], right: &[u32]) -> Alignment {
    let prefix = left.iter().zip(right).take_while(|(x, y)| x == y).count();
    let suffix = left[prefix..]
        .iter()
        .rev()
        .zip(right[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let tokens = |side: &[u32]| -> Vec<Token> { side.iter().copied().map(Token).collect() };
    let lt = tokens(&left[prefix..left.len() - suffix]);
    let rt = tokens(&right[prefix..right.len() - suffix]);
    let mut diff = Diff::default();
    diff.compute_with(algorithm, &lt, &rt, num_tokens(left, right));
    let mut out = Alignment {
        removed: vec![false; left.len()],
        added: vec![false; right.len()],
    };
    for i in 0..lt.len() {
        out.removed[prefix + i] = diff.is_removed(u32::try_from(i).unwrap());
    }
    for j in 0..rt.len() {
        out.added[prefix + j] = diff.is_added(u32::try_from(j).unwrap());
    }
    out
}

/// `n` copies of one line, and a copy with `edits` lines replaced by unique ones.
fn identical_with_edits(n: usize, edits: u32) -> (Vec<u32>, Vec<u32>) {
    let left = vec![0u32; n];
    let mut right = left.clone();
    for k in 1..=edits {
        right[n / (edits as usize + 1) * k as usize] = k;
    }
    (left, right)
}

/// `n` unique lines, and a copy with every adjacent pair swapped.
fn swapped_unique(n: u32) -> (Vec<u32>, Vec<u32>) {
    let left: Vec<u32> = (0..n).collect();
    let mut right = left.clone();
    swap_pairs(&mut right);
    (left, right)
}

fn swap_pairs(side: &mut [u32]) {
    for pair in side.as_chunks_mut::<2>().0 {
        pair.swap(0, 1);
    }
}

/// `n` lines of `n / copies` distinct keys in shuffled order (each occurs
/// `copies` times), and a copy with every adjacent pair swapped.
fn low_frequency(n: usize, copies: usize) -> (Vec<u32>, Vec<u32>) {
    let mut rng = Rng(0x5EED_1234);
    let mut left: Vec<u32> = (0..n).map(|i| u32::try_from(i / copies).unwrap()).collect();
    for i in (1..n).rev() {
        left.swap(i, rng.below(i + 1));
    }
    let mut right = left.clone();
    swap_pairs(&mut right);
    (left, right)
}

fn histogram(l: &[u32], r: &[u32]) -> (Alignment, AlignReport) {
    align_with_report(LineAlgorithm::Histogram, l, r, num_tokens(l, r))
}

/// The work counter can pass the budget by at most one region's worth of
/// steps before it is checked.
fn assert_bounded_work(report: &AlignReport, lines: usize) {
    let lines = lines as u64;
    assert!(
        report.histogram_work <= report.histogram_budget + 2 * lines,
        "work {} over budget {}",
        report.histogram_work,
        report.histogram_budget
    );
}

#[test]
fn five_million_identical_lines_with_ten_edits() {
    // imara-diff 0.2.0 did not finish this in 120 s (release). It takes about
    // 0.3 s per algorithm in release and a few seconds in debug.
    let (l, r) = identical_with_edits(5_000_000, 10);
    for alg in [LineAlgorithm::Histogram, LineAlgorithm::Myers] {
        let ((a, report), t) = timed(|| align_with_report(alg, &l, &r, num_tokens(&l, &r)));
        assert_valid(&l, &r, &a);
        assert_eq!(changed(&a), 20, "{alg:?}: ten one-line replacements");
        assert!(!is_approximate(&l, &r, &a), "{alg:?}");
        assert!(!report.histogram_fell_back);
        assert!(t < Duration::from_secs(60), "{alg:?} took {t:?}");
    }
    // Patience (`similar`) at a tenth of the size, to keep the debug run short.
    let (l, r) = identical_with_edits(500_000, 10);
    let a = align(LineAlgorithm::Patience, &l, &r, num_tokens(&l, &r));
    assert_valid(&l, &r, &a);
    assert_eq!(changed(&a), 20);
}

#[test]
fn swapped_pairs_hit_the_budget_and_fall_back() {
    let (l, r) = swapped_unique(20_000);
    let (a, report) = histogram(&l, &r);
    assert_valid(&l, &r, &a);
    assert!(report.histogram_fell_back);
    assert_eq!(report.histogram_budget, histogram_budget(l.len(), r.len()));
    assert_bounded_work(&report, l.len() + r.len());
    // Every changed line occurs on the other side, so this is flagged.
    assert!(is_approximate(&l, &r, &a));
    // Deterministic: a second run gives the same result and the same count.
    assert_eq!(histogram(&l, &r), (a, report));
}

#[test]
fn work_grows_linearly_on_swapped_pairs() {
    // Unguarded, the work here grows with the square of the size. Guarded,
    // it is capped by the budget, which is linear.
    let mut per_line = Vec::new();
    // Sizes above the budget floor (HISTOGRAM_WORK_MIN).
    for n in [16_000u32, 24_000, 32_000] {
        let (l, r) = swapped_unique(n);
        let (a, report) = histogram(&l, &r);
        assert_valid(&l, &r, &a);
        assert_bounded_work(&report, l.len() + r.len());
        per_line.push(report.histogram_work / (2 * u64::from(n)));
    }
    assert!(
        per_line
            .iter()
            .all(|w| *w <= line_diff::HISTOGRAM_WORK_PER_LINE + 2)
    );
}

#[test]
fn unguarded_swapped_pairs_are_quadratic() {
    // The case the budget exists for: unguarded work is far above it.
    let (l, r) = swapped_unique(4_000);
    let (_, free) = align_histogram_with_budget(&l, &r, num_tokens(&l, &r), u64::MAX);
    assert!(free.histogram_work > 4_000 * 4_000 / 2);
    assert!(!free.histogram_fell_back);
}

#[test]
fn low_frequency_lines_match_upstream_histogram() {
    // Every line occurs 16 times (below Histogram's 64-occurrence cut-off),
    // at the spike size of 200 k lines. Fast, and under the budget.
    let (l, r) = low_frequency(200_000, 16);
    let ((a, report), t) = timed(|| histogram(&l, &r));
    assert_valid(&l, &r, &a);
    assert!(!report.histogram_fell_back);
    assert_eq!(a, upstream(Algorithm::Histogram, &l, &r));
    assert!(t < Duration::from_secs(20), "took {t:?}");
    // Histogram is far from minimal here (a known weakness shared with git);
    // Myers finds about half the edits.
    let (l, r) = low_frequency(20_000, 16);
    let a = histogram(&l, &r).0;
    let myers = align(LineAlgorithm::Myers, &l, &r, num_tokens(&l, &r));
    assert_valid(&l, &r, &myers);
    assert!(changed(&myers) < changed(&a));
}

#[test]
fn all_different_lines() {
    let n = 200_000u32;
    let l: Vec<u32> = (0..n).collect();
    let r: Vec<u32> = (n..2 * n).collect();
    for alg in ALL {
        let (a, _) = align_with_report(alg, &l, &r, 2 * n);
        assert!(a.removed.iter().all(|x| *x) && a.added.iter().all(|x| *x));
        assert!(!is_approximate(&l, &r, &a), "{alg:?}");
    }
    let (_, report) = histogram(&l, &r);
    assert!(!report.histogram_fell_back);
    // One populate pass over the left side and one scan of the right side.
    assert_eq!(report.histogram_work, 2 * u64::from(n));
}

#[test]
fn outputs_are_valid_and_myers_is_minimal_when_not_approximate() {
    let mut rng = Rng(0xABCD_EF01);
    let mut exact = 0;
    for round in 0..120 {
        let n = 20 + rng.below(600);
        let alphabet = [2, 5, 30, 300, 5000][round % 5];
        let l: Vec<u32> = (0..n)
            .map(|_| u32::try_from(rng.below(alphabet)).unwrap())
            .collect();
        let mut r = l.clone();
        for _ in 0..rng.below(80) {
            let at = rng.below(r.len() + 1);
            match rng.below(3) {
                0 if at < r.len() => {
                    r.remove(at);
                }
                1 if at < r.len() => r[at] = u32::try_from(rng.below(alphabet)).unwrap(),
                _ => r.insert(at, u32::try_from(rng.below(alphabet)).unwrap()),
            }
        }
        for alg in ALL {
            assert_valid(&l, &r, &align(alg, &l, &r, num_tokens(&l, &r)));
        }
        // Histogram matches upstream below the budget, and is Myers past it.
        let (h, report) = histogram(&l, &r);
        assert!(!report.histogram_fell_back);
        assert_eq!(h, upstream(Algorithm::Histogram, &l, &r), "round {round}");
        let (forced, _) = align_histogram_with_budget(&l, &r, num_tokens(&l, &r), 0);
        let myers = align(LineAlgorithm::Myers, &l, &r, num_tokens(&l, &r));
        assert_eq!(forced, myers, "round {round}");
        if !is_approximate(&l, &r, &myers) {
            let minimal = upstream(Algorithm::MyersMinimal, &l, &r);
            assert_eq!(changed(&myers), changed(&minimal), "round {round}");
            exact += 1;
        }
    }
    assert!(exact > 50);
}

/// Times `f`, returning its result and the elapsed time.
fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let started = Instant::now();
    let out = f();
    (out, started.elapsed())
}

#[test]
#[ignore = "full size; run in release with --ignored"]
fn release_swapped_pairs_take_about_the_time_of_myers() {
    // Unguarded Histogram took 33 s here (spike A); Myers about 0.4 s.
    let (left, right) = swapped_unique(200_000);
    let tokens = num_tokens(&left, &right);
    let ((guarded, report), histogram_time) =
        timed(|| align_with_report(LineAlgorithm::Histogram, &left, &right, tokens));
    let (myers, myers_time) = timed(|| align(LineAlgorithm::Myers, &left, &right, tokens));
    assert_valid(&left, &right, &guarded);
    assert_valid(&left, &right, &myers);
    assert!(report.histogram_fell_back);
    assert!(
        histogram_time < Duration::from_secs(1),
        "histogram took {histogram_time:?}"
    );
    assert!(
        histogram_time < 2 * myers_time + Duration::from_millis(250),
        "histogram {histogram_time:?}, myers {myers_time:?}"
    );
}
