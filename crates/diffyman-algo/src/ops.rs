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
}

impl OpKind {
    /// True for `insert`, `delete` and `replace`.
    pub fn is_change(self) -> bool {
        self != OpKind::Equal
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

/// True for a change op whose lines are all blank (empty or whitespace only).
///
/// This is the building block for `ignore.blankLines`. The Diff Document
/// 0.1.0 defers that option (a blank-only change has different line counts on
/// each side, so it cannot be shown as an `equal` op), so the engine rejects
/// it for now.
pub fn is_blank_change(op: &LineOp, left: &[&str], right: &[&str]) -> bool {
    op.kind.is_change()
        && left[op.left_start..op.left_start + op.left_len]
            .iter()
            .chain(&right[op.right_start..op.right_start + op.right_len])
            .all(|line| crate::key::is_blank(line))
}

/// Groups the full op list into hunks with `context` lines of context.
///
/// Two consecutive change ops share a hunk when the equal op between them has
/// at most `2 * context` lines (it is then included whole). Leading and
/// trailing equal ops are clipped to `context` lines; empty ops are dropped.
pub fn group_hunks(ops: &[LineOp], context: usize) -> Vec<Vec<LineOp>> {
    let changes: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.kind.is_change())
        .map(|(i, _)| i)
        .collect();
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for &c in &changes {
        if let Some(last) = groups.last_mut() {
            let between = &ops[last.1 + 1..c];
            let gap: usize = between.iter().map(|op| op.left_len).sum();
            if gap <= context.saturating_mul(2) {
                last.1 = c;
                continue;
            }
        }
        groups.push((c, c));
    }
    let mut hunks = Vec::with_capacity(groups.len());
    for (first, last) in groups {
        let mut hunk = Vec::new();
        if first > 0 {
            let eq = ops[first - 1];
            let take = context.min(eq.left_len);
            if take > 0 {
                hunk.push(LineOp {
                    kind: OpKind::Equal,
                    left_start: eq.left_start + eq.left_len - take,
                    left_len: take,
                    right_start: eq.right_start + eq.right_len - take,
                    right_len: take,
                });
            }
        }
        hunk.extend_from_slice(&ops[first..=last]);
        if last + 1 < ops.len() {
            let eq = ops[last + 1];
            let take = context.min(eq.left_len);
            if take > 0 {
                hunk.push(LineOp {
                    kind: OpKind::Equal,
                    left_start: eq.left_start,
                    left_len: take,
                    right_start: eq.right_start,
                    right_len: take,
                });
            }
        }
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
