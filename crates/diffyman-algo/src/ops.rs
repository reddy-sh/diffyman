//! Full op list and hunk grouping (Diff Document spec §7.3 and §7.4).
//!
//! Positions here are 0-based line indices; the engine converts them to the
//! 1-based ranges of the Diff Document.

use crate::line_diff::Alignment;

/// Op kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    /// Matched lines; equal non-zero counts on both sides.
    Equal,
    /// Right lines only.
    Insert,
    /// Left lines only.
    Delete,
    /// Lines on both sides.
    Replace,
    /// A change whose lines are all blank, under `ignore.blankLines`. Counts
    /// may differ and either may be 0. Not a change op.
    Ignored,
}

impl OpKind {
    /// True for `insert`, `delete` and `replace`.
    pub fn is_change(self) -> bool {
        matches!(self, OpKind::Insert | OpKind::Delete | OpKind::Replace)
    }
}

/// One op over 0-based, half-open line ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineOp {
    /// Op kind.
    pub kind: OpKind,
    /// First left line.
    pub left_start: usize,
    /// Number of left lines.
    pub left_len: usize,
    /// First right line.
    pub right_start: usize,
    /// Number of right lines.
    pub right_len: usize,
}

impl LineOp {
    fn change(left_start: usize, left_len: usize, right_start: usize, right_len: usize) -> LineOp {
        let kind = match (left_len > 0, right_len > 0) {
            (true, true) => OpKind::Replace,
            (true, false) => OpKind::Delete,
            _ => OpKind::Insert,
        };
        LineOp {
            kind,
            left_start,
            left_len,
            right_start,
            right_len,
        }
    }
}

/// Builds the full op list, which tiles both files. Each maximal run of matched
/// lines is one `equal` op; each maximal run of unmatched lines between two
/// equal ops (or a file edge) is one change op, so change ops are never adjacent.
pub fn full_ops(alignment: &Alignment) -> Vec<LineOp> {
    let (removed, added) = (&alignment.removed, &alignment.added);
    let (n, m) = (removed.len(), added.len());
    let (mut i, mut j) = (0, 0);
    let mut ops: Vec<LineOp> = Vec::new();
    while i < n || j < m {
        let (ei, ej) = (i, j);
        while i < n && j < m && !removed[i] && !added[j] {
            i += 1;
            j += 1;
        }
        if i > ei {
            ops.push(LineOp {
                kind: OpKind::Equal,
                left_start: ei,
                left_len: i - ei,
                right_start: ej,
                right_len: j - ej,
            });
        }
        let (ci, cj) = (i, j);
        loop {
            let (si, sj) = (i, j);
            while i < n && removed[i] {
                i += 1;
            }
            while j < m && added[j] {
                j += 1;
            }
            if (i, j) == (si, sj) {
                break;
            }
        }
        if (i, j) == (ei, ej) {
            // No progress: the alignment is inconsistent (one side ran out of
            // matched lines). Treat the rest as one change.
            push_change(&mut ops, i, n - i, j, m - j);
            break;
        }
        push_change(&mut ops, ci, i - ci, cj, j - cj);
    }
    ops
}

fn push_change(ops: &mut Vec<LineOp>, ls: usize, ll: usize, rs: usize, rl: usize) {
    if ll == 0 && rl == 0 {
        return;
    }
    if let Some(last) = ops.last_mut()
        && last.kind.is_change()
    {
        *last = LineOp::change(
            last.left_start,
            last.left_len + ll,
            last.right_start,
            last.right_len + rl,
        );
        return;
    }
    ops.push(LineOp::change(ls, ll, rs, rl));
}

/// True for a change op whose lines are all blank (empty or whitespace only),
/// on both sides. The terminators do not matter.
pub fn is_blank_change(op: &LineOp, left: &[&str], right: &[&str]) -> bool {
    op.kind.is_change()
        && left[op.left_start..op.left_start + op.left_len]
            .iter()
            .chain(&right[op.right_start..op.right_start + op.right_len])
            .all(|line| crate::key::is_blank(line))
}

/// `ignore.blankLines` (Diff Document spec §7.3 step 3): turns every change op
/// whose lines are all blank into an [`OpKind::Ignored`] op. `left` and
/// `right` are the line contents without terminators.
pub fn ignore_blank_changes(ops: &mut [LineOp], left: &[&str], right: &[&str]) {
    for op in ops.iter_mut() {
        if is_blank_change(op, left, right) {
            op.kind = OpKind::Ignored;
        }
    }
}

/// Groups the full op list into hunks with `context` lines of context
/// (Diff Document spec §7.4).
///
/// Only `equal` lines count as context; `ignored` ops are kept whole and do
/// not count. Two consecutive change ops share a hunk when the ops between
/// them hold at most `2 * context` equal lines (they are then included whole).
/// Before the first and after the last change op of a hunk, ops are taken
/// outward until `context` equal lines are reached: equal ops are clipped,
/// ignored ops are taken whole, and empty ops are dropped.
pub fn group_hunks(ops: &[LineOp], context: usize) -> Vec<Vec<LineOp>> {
    let equal_lines = |run: &[LineOp]| -> usize {
        run.iter()
            .filter(|op| op.kind == OpKind::Equal)
            .map(|op| op.left_len)
            .sum()
    };
    let changes: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.kind.is_change())
        .map(|(i, _)| i)
        .collect();
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for &c in &changes {
        if let Some(last) = groups.last_mut()
            && equal_lines(&ops[last.1 + 1..c]) <= context.saturating_mul(2)
        {
            last.1 = c;
            continue;
        }
        groups.push((c, c));
    }
    let mut hunks = Vec::with_capacity(groups.len());
    for (first, last) in groups {
        let mut hunk = Vec::new();
        // Leading context, collected backwards.
        let mut budget = context;
        let mut i = first;
        while i > 0 && budget > 0 && !ops[i - 1].kind.is_change() {
            i -= 1;
            let op = ops[i];
            if op.kind == OpKind::Equal {
                let take = budget.min(op.left_len);
                budget -= take;
                hunk.push(LineOp {
                    kind: OpKind::Equal,
                    left_start: op.left_start + op.left_len - take,
                    left_len: take,
                    right_start: op.right_start + op.right_len - take,
                    right_len: take,
                });
            } else {
                hunk.push(op);
            }
        }
        hunk.reverse();
        hunk.extend_from_slice(&ops[first..=last]);
        // Trailing context.
        let mut budget = context;
        let mut i = last + 1;
        while i < ops.len() && budget > 0 && !ops[i].kind.is_change() {
            let op = ops[i];
            i += 1;
            if op.kind == OpKind::Equal {
                let take = budget.min(op.left_len);
                budget -= take;
                hunk.push(LineOp {
                    kind: OpKind::Equal,
                    left_start: op.left_start,
                    left_len: take,
                    right_start: op.right_start,
                    right_len: take,
                });
            } else {
                hunk.push(op);
            }
        }
        hunk.retain(|op| op.left_len + op.right_len > 0);
        hunks.push(hunk);
    }
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alignment(removed: &[u8], added: &[u8]) -> Alignment {
        Alignment {
            removed: removed.iter().map(|b| *b == 1).collect(),
            added: added.iter().map(|b| *b == 1).collect(),
        }
    }

    fn kinds(ops: &[LineOp]) -> Vec<OpKind> {
        ops.iter().map(|o| o.kind).collect()
    }

    #[test]
    fn worked_example_ops() {
        // Spec §14: equal L1/R1, replace L2/R2, equal L3-5/R3-5, insert R6.
        let a = alignment(&[0, 1, 0, 0, 0], &[0, 1, 0, 0, 0, 1]);
        let ops = full_ops(&a);
        assert_eq!(
            kinds(&ops),
            [
                OpKind::Equal,
                OpKind::Replace,
                OpKind::Equal,
                OpKind::Insert
            ]
        );
        assert_eq!(
            ops[3],
            LineOp {
                kind: OpKind::Insert,
                left_start: 5,
                left_len: 0,
                right_start: 5,
                right_len: 1
            }
        );
        let hunks = group_hunks(&ops, 1);
        assert_eq!(hunks.len(), 2);
        assert_eq!(
            kinds(&hunks[0]),
            [OpKind::Equal, OpKind::Replace, OpKind::Equal]
        );
        assert_eq!((hunks[0][2].left_start, hunks[0][2].left_len), (2, 1));
        assert_eq!(kinds(&hunks[1]), [OpKind::Equal, OpKind::Insert]);
        assert_eq!((hunks[1][0].left_start, hunks[1][0].left_len), (4, 1));
        // With context 2, the 3-line gap is <= 4, so one hunk holds everything.
        let hunks = group_hunks(&ops, 2);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].len(), 4);
    }

    #[test]
    fn interleaved_changes_merge_into_one_op() {
        // Deletions and insertions with no matched line between them form one replace.
        let a = alignment(&[1, 1, 0], &[1, 0]);
        let ops = full_ops(&a);
        assert_eq!(kinds(&ops), [OpKind::Replace, OpKind::Equal]);
        assert_eq!((ops[0].left_len, ops[0].right_len), (2, 1));
    }

    #[test]
    fn delete_and_insert_only() {
        let ops = full_ops(&alignment(&[1, 1], &[]));
        assert_eq!(kinds(&ops), [OpKind::Delete]);
        let ops = full_ops(&alignment(&[], &[1]));
        assert_eq!(kinds(&ops), [OpKind::Insert]);
        assert!(full_ops(&alignment(&[], &[])).is_empty());
    }

    #[test]
    fn zero_context() {
        let a = alignment(&[1, 0, 1], &[0, 0, 1]);
        let ops = full_ops(&a);
        let hunks = group_hunks(&ops, 0);
        assert_eq!(
            hunks.len(),
            ops.iter().filter(|o| o.kind.is_change()).count()
        );
        assert!(hunks.iter().all(|h| h.len() == 1 && h[0].kind.is_change()));
    }

    fn op(kind: OpKind, ls: usize, ll: usize, rs: usize, rl: usize) -> LineOp {
        LineOp {
            kind,
            left_start: ls,
            left_len: ll,
            right_start: rs,
            right_len: rl,
        }
    }

    #[test]
    fn ignored_ops_are_context_that_does_not_count() {
        use OpKind::{Equal, Ignored, Insert, Replace};
        // equal 2, ignored (+1 right), equal 1, replace, equal 1, ignored
        // (-2 left), equal 3, insert.
        let ops = [
            op(Equal, 0, 2, 0, 2),
            op(Ignored, 2, 0, 2, 1),
            op(Equal, 2, 1, 3, 1),
            op(Replace, 3, 1, 4, 1),
            op(Equal, 4, 1, 5, 1),
            op(Ignored, 5, 2, 6, 0),
            op(Equal, 7, 3, 6, 3),
            op(Insert, 10, 0, 9, 1),
        ];
        // Context 2: four equal lines lie between the two changes (the ignored
        // lines do not count), so they share a hunk. The leading context takes
        // one equal line, the ignored op whole, and one more equal line.
        let hunks = group_hunks(&ops, 2);
        assert_eq!(hunks.len(), 1);
        assert_eq!(
            kinds(&hunks[0]),
            [
                Equal, Ignored, Equal, Replace, Equal, Ignored, Equal, Insert
            ]
        );
        assert_eq!(hunks[0][0], op(Equal, 1, 1, 1, 1));
        // Context 1: the gap of 4 equal lines is above 2, so two hunks. The
        // leading context stops at one equal line, before the ignored op.
        let hunks = group_hunks(&ops, 1);
        assert_eq!(hunks.len(), 2);
        assert_eq!(kinds(&hunks[0]), [Equal, Replace, Equal]);
        assert_eq!(kinds(&hunks[1]), [Equal, Insert]);
        // Context 0: change ops only.
        let hunks = group_hunks(&ops, 0);
        assert!(hunks.iter().all(|h| h.len() == 1 && h[0].kind.is_change()));
        // Context 3: the trailing context of the replace takes the ignored op
        // whole and then two lines of the next equal op.
        let hunks = group_hunks(&ops[..7], 3);
        assert_eq!(
            hunks[0][3..],
            [
                op(Replace, 3, 1, 4, 1),
                op(Equal, 4, 1, 5, 1),
                op(Ignored, 5, 2, 6, 0),
                op(Equal, 7, 2, 6, 2)
            ]
        );
        // Only ignored ops: no hunks.
        let only = [
            op(Equal, 0, 1, 0, 1),
            op(Ignored, 1, 1, 1, 0),
            op(Equal, 2, 1, 1, 1),
        ];
        assert!(group_hunks(&only, 3).is_empty());
    }

    #[test]
    fn blank_changes_become_ignored() {
        let left = ["a", "", "b", "", "c"];
        let right = ["a", " ", "\t", "b", "x", "c"];
        let mut ops = full_ops(&alignment(&[0, 1, 0, 1, 0], &[0, 1, 1, 0, 1, 0]));
        assert_eq!(
            kinds(&ops),
            [
                OpKind::Equal,
                OpKind::Replace,
                OpKind::Equal,
                OpKind::Replace,
                OpKind::Equal
            ]
        );
        ignore_blank_changes(&mut ops, &left, &right);
        // L2 -> R2-3 is blank on both sides; L4 -> R5 replaces a blank line with text.
        assert_eq!(
            kinds(&ops),
            [
                OpKind::Equal,
                OpKind::Ignored,
                OpKind::Equal,
                OpKind::Replace,
                OpKind::Equal
            ]
        );
        assert!(!OpKind::Ignored.is_change());
    }

    #[test]
    fn blank_changes() {
        let left = ["a", "", "b"];
        let right = ["a", "  ", "\t", "b", "c"];
        let ops = full_ops(&alignment(&[0, 1, 0], &[0, 1, 1, 0, 1]));
        assert!(is_blank_change(&ops[1], &left, &right));
        assert!(!is_blank_change(&ops[3], &left, &right));
        assert!(!is_blank_change(&ops[0], &left, &right));
    }

    #[test]
    fn inconsistent_alignment_still_tiles() {
        // Two matched left lines but one matched right line.
        let a = alignment(&[0, 0], &[0]);
        let ops = full_ops(&a);
        let left: usize = ops.iter().map(|o| o.left_len).sum();
        let right: usize = ops.iter().map(|o| o.right_len).sum();
        assert_eq!((left, right), (2, 1));
    }
}
