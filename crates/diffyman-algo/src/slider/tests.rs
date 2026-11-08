use super::*;
use crate::line_diff::{self, LineAlgorithm};
use crate::ops::{OpKind, full_ops};

/// Lines, their tokens (equal text, equal token) and contents.
struct Doc {
    lines: Vec<String>,
    tokens: Vec<u32>,
}

fn docs<'s>(left: &[&'s str], right: &[&'s str]) -> (Doc, Doc) {
    let mut ids: Vec<&'s str> = Vec::new();
    let mut token = |s: &'s str| -> u32 {
        let found = ids.iter().position(|x| *x == s);
        let id = found.unwrap_or_else(|| {
            ids.push(s);
            ids.len() - 1
        });
        u32::try_from(id).unwrap()
    };
    let l: Vec<u32> = left.iter().map(|s| token(s)).collect();
    let r: Vec<u32> = right.iter().map(|s| token(s)).collect();
    (
        Doc {
            lines: left.iter().map(|s| (*s).to_owned()).collect(),
            tokens: l,
        },
        Doc {
            lines: right.iter().map(|s| (*s).to_owned()).collect(),
            tokens: r,
        },
    )
}

fn side<'d>(doc: &'d Doc) -> TokenSide<'d, impl Fn(usize) -> &'d [u8] + 'd> {
    TokenSide {
        tokens: &doc.tokens,
        content: move |i: usize| doc.lines[i].as_bytes(),
    }
}

fn flags(bits: &[u8]) -> Vec<bool> {
    bits.iter().map(|b| *b == 1).collect()
}

fn run_compact(l: &Doc, r: &Doc, a: &mut Alignment, budget: u64) -> CompactReport {
    compact(a, &mut side(l), &mut side(r), budget)
}

/// Matched lines pair up in order and have equal tokens.
fn assert_valid(l: &[u32], r: &[u32], a: &Alignment) {
    let lm: Vec<u32> = l
        .iter()
        .zip(&a.removed)
        .filter(|(_, x)| !**x)
        .map(|(t, _)| *t)
        .collect();
    let rm: Vec<u32> = r
        .iter()
        .zip(&a.added)
        .filter(|(_, x)| !**x)
        .map(|(t, _)| *t)
        .collect();
    assert_eq!(lm, rm);
}

#[test]
fn deletion_lines_up_with_insertion_below() {
    let (l, r) = docs(&["k", "a", "a", "c"], &["k", "a", "X", "c"]);
    // Left line 1 deleted, right line 2 inserted: two separate changes.
    let mut a = Alignment {
        removed: flags(&[0, 1, 0, 0]),
        added: flags(&[0, 0, 1, 0]),
    };
    run_compact(&l, &r, &mut a, u64::MAX);
    assert_eq!(a.removed, flags(&[0, 0, 1, 0]));
    let kinds: Vec<OpKind> = full_ops(&a).iter().map(|o| o.kind).collect();
    assert_eq!(kinds, [OpKind::Equal, OpKind::Replace, OpKind::Equal]);
}

#[test]
fn deletion_lines_up_with_insertion_above() {
    let (l, r) = docs(&["k", "a", "a", "c"], &["k", "X", "a", "c"]);
    let mut a = Alignment {
        removed: flags(&[0, 0, 1, 0]),
        added: flags(&[0, 1, 0, 0]),
    };
    run_compact(&l, &r, &mut a, u64::MAX);
    assert_eq!(a.removed, flags(&[0, 1, 0, 0]));
    assert_eq!(full_ops(&a).len(), 3);
}

/// A function inserted between two others: the blank line between them can
/// go at either end of the insertion. The heuristic puts the insertion after
/// the blank line, so it starts with `fn b() {` and ends with its own blank.
#[test]
fn inserted_function_starts_at_its_header() {
    let left = ["fn a() {", "    x", "}", "", "fn c() {", "    z", "}"];
    let right = [
        "fn a() {", "    x", "}", "", "fn b() {", "    y", "}", "", "fn c() {", "    z", "}",
    ];
    let (l, r) = docs(&left, &right);
    // As an algorithm may report it: `}`, blank, `fn b() {`, `    y` (lines 2..6).
    let mut a = Alignment {
        removed: vec![false; 7],
        added: flags(&[0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0]),
    };
    run_compact(&l, &r, &mut a, u64::MAX);
    assert_eq!(a.added, flags(&[0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0]));
    assert_valid(&l.tokens, &r.tokens, &a);
}

/// Without indentation or blank lines to go by, a run ends up at its lowest
/// position (ties go down).
#[test]
fn flat_text_slides_down() {
    let (l, r) = docs(&["a", "b", "c"], &["a", "b", "a", "b", "c"]);
    let mut a = Alignment {
        removed: vec![false; 3],
        added: flags(&[1, 1, 0, 0, 0]),
    };
    run_compact(&l, &r, &mut a, u64::MAX);
    assert_eq!(a.added, flags(&[0, 0, 1, 1, 0]));
}

/// A run that merges with its neighbour while sliding is slid again.
#[test]
fn merged_runs_slide_together() {
    let (l, r) = docs(&["a", "c"], &["a", "b", "a", "b", "c"]);
    // Right `a b` (0..2) and `b` (3) inserted, around the matched `a`.
    let mut a = Alignment {
        removed: vec![false; 2],
        added: flags(&[1, 1, 0, 1, 0]),
    };
    run_compact(&l, &r, &mut a, u64::MAX);
    // `a b` slides down one line and touches `b`: one run `b a b`.
    assert_eq!(a.added, flags(&[0, 1, 1, 1, 0]));
    assert_valid(&l.tokens, &r.tokens, &a);
}

#[test]
fn zero_budget_changes_nothing() {
    let (l, r) = docs(&["a", "b", "c"], &["a", "b", "a", "b", "c"]);
    let before = Alignment {
        removed: vec![false; 3],
        added: flags(&[1, 1, 0, 0, 0]),
    };
    let mut a = before.clone();
    let report = run_compact(&l, &r, &mut a, 0);
    assert!(report.exhausted);
    assert_eq!(a, before);
}

#[test]
fn inconsistent_alignment_is_left_alone() {
    let (l, r) = docs(&["a", "a"], &["a"]);
    let before = Alignment {
        removed: flags(&[0, 0]),
        added: flags(&[0]),
    };
    let mut a = before.clone();
    run_compact(&l, &r, &mut a, u64::MAX);
    assert_eq!(a, before);
}

#[test]
fn indentation() {
    assert_eq!(indent(b""), None);
    assert_eq!(indent(b" \t "), None);
    assert_eq!(indent(b"x"), Some(0));
    assert_eq!(indent(b"  x"), Some(2));
    assert_eq!(indent(b"\tx"), Some(8));
    assert_eq!(indent(b"   \tx"), Some(8));
    assert_eq!(indent(b"        \t x"), Some(17));
    // Form feed and vertical tab are not indentation.
    assert_eq!(indent(b"\x0cx"), Some(0));
    assert_eq!(indent(b"  \x0b"), Some(2));
    let deep = " ".repeat(500) + "x";
    assert_eq!(indent(deep.as_bytes()), Some(MAX_INDENT));
    let blank = " ".repeat(500);
    assert_eq!(indent(blank.as_bytes()), Some(MAX_INDENT));
}

/// Small deterministic generator (xorshift64*).
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

/// Random code-like lines from a small vocabulary, so sliders are common.
#[test]
fn random_alignments_stay_valid() {
    const VOCAB: [&str; 8] = ["", "}", "    x();", "fn f() {", "  y", "\tz", "a", "b"];
    let mut rng = Rng(0x5EED_51DE);
    for round in 0..300 {
        let size = 5 + (round % 60);
        let left: Vec<&str> = (0..size).map(|_| VOCAB[rng.next(8) as usize]).collect();
        let mut right = left.clone();
        for _ in 0..=(round % 7) {
            let at = rng.next(u32::try_from(right.len()).unwrap() + 1) as usize;
            if rng.next(2) == 0 && at < right.len() {
                right.remove(at);
            } else {
                right.insert(at, VOCAB[rng.next(8) as usize]);
            }
        }
        let (ldoc, rdoc) = docs(&left, &right);
        for alg in [
            LineAlgorithm::Histogram,
            LineAlgorithm::Myers,
            LineAlgorithm::Patience,
        ] {
            let before = line_diff::align(alg, &ldoc.tokens, &rdoc.tokens, 8);
            let mut got = before.clone();
            let report = run_compact(&ldoc, &rdoc, &mut got, u64::MAX);
            assert!(!report.exhausted);
            assert_valid(&ldoc.tokens, &rdoc.tokens, &got);
            let count = |f: &[bool]| f.iter().filter(|x| **x).count();
            assert_eq!(count(&got.removed), count(&before.removed), "round {round}");
            assert_eq!(count(&got.added), count(&before.added), "round {round}");
            // The same multiset of changed keys.
            let changed = |t: &[u32], f: &[bool]| -> Vec<u32> {
                let mut keys: Vec<u32> = t
                    .iter()
                    .zip(f)
                    .filter(|(_, x)| **x)
                    .map(|(t, _)| *t)
                    .collect();
                keys.sort_unstable();
                keys
            };
            assert_eq!(
                changed(&ldoc.tokens, &got.removed),
                changed(&ldoc.tokens, &before.removed)
            );
            assert_eq!(
                changed(&rdoc.tokens, &got.added),
                changed(&rdoc.tokens, &before.added)
            );
        }
    }
}

/// Many copies of one line with one more inserted: the run slides over all
/// of them once each way, and the work stays linear.
#[test]
fn repeated_lines_cost_linear_work() {
    let n = 200_000;
    let left: Vec<u32> = vec![0; n];
    let right: Vec<u32> = vec![0; n + 1];
    let mut a = line_diff::align(LineAlgorithm::Histogram, &left, &right, 1);
    let content = |_: usize| -> &[u8] { b"same line" };
    let report = compact(
        &mut a,
        &mut TokenSide {
            tokens: &left,
            content,
        },
        &mut TokenSide {
            tokens: &right,
            content,
        },
        compaction_budget(n, n + 1),
    );
    assert!(!report.exhausted);
    assert!(report.work < 8 * (2 * n as u64 + 1), "work {}", report.work);
    assert_eq!(a.added.iter().filter(|x| **x).count(), 1);
}
