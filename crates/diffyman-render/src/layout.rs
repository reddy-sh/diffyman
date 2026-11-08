//! Hunks to render: the document's own hunks, or the same changes regrouped
//! with another number of context lines (spec §7.4 applied to the ops the
//! document holds).

use diffyman_model::{DiffDocument, OpKind, Span};

use crate::source::range0;

/// One op with 0-based line positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ROp<'d> {
    pub kind: OpKind,
    /// First left line (0-based) and count.
    pub l: usize,
    pub ln: usize,
    /// First right line (0-based) and count.
    pub r: usize,
    pub rn: usize,
    /// The document op's inline spans (only `replace` ops have them).
    pub inline: &'d [Span],
}

/// A hunk to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RHunk<'d> {
    pub ops: Vec<ROp<'d>>,
}

impl RHunk<'_> {
    /// Left `(start, count)`, 0-based.
    pub fn left(&self) -> (usize, usize) {
        (self.ops[0].l, self.ops.iter().map(|o| o.ln).sum())
    }

    /// Right `(start, count)`, 0-based.
    pub fn right(&self) -> (usize, usize) {
        (self.ops[0].r, self.ops.iter().map(|o| o.rn).sum())
    }
}

/// An op, or lines outside every hunk that cannot be shown as context.
#[derive(Debug, Clone, Copy)]
enum Item<'d> {
    Op(ROp<'d>),
    /// Lines between hunks that may hold `ignored` ops: context never crosses them.
    Barrier,
}

/// The hunks to render. `context: None` (or the document's own value) keeps
/// the document's hunks. Another value regroups the change ops:
///
/// - Fewer lines than the document's context: exact, because every hunk
///   already holds the lines needed.
/// - More lines: the lines between hunks are unchanged lines, unless the
///   document used `ignore.blankLines`, where they may hold `ignored` ops that
///   the document does not list; context then stops at the document's hunks.
///
/// The document must have passed [`crate::source::load`].
pub(crate) fn hunks<'d>(
    doc: &'d DiffDocument,
    nl: usize,
    nr: usize,
    context: Option<u32>,
) -> Vec<RHunk<'d>> {
    let own: Vec<RHunk<'d>> = doc
        .hunks
        .iter()
        .map(|h| RHunk {
            ops: h
                .ops
                .iter()
                .map(|o| {
                    let (l, ln) = range0(o.left_range, nl).unwrap_or((0, 0));
                    let (r, rn) = range0(o.right_range, nr).unwrap_or((0, 0));
                    ROp {
                        kind: o.op,
                        l,
                        ln,
                        r,
                        rn,
                        inline: &o.inline,
                    }
                })
                .collect(),
        })
        .collect();
    match context {
        Some(c) if c != doc.options.context => {
            regroup(&own, nl, nr, c, doc.options.ignore.blank_lines)
        }
        _ => own,
    }
}

fn regroup<'d>(
    own: &[RHunk<'d>],
    nl: usize,
    nr: usize,
    context: u32,
    blank_lines: bool,
) -> Vec<RHunk<'d>> {
    let mut items: Vec<Item<'d>> = Vec::new();
    let (mut cl, mut cr) = (0usize, 0usize);
    let gap = |items: &mut Vec<Item<'d>>, l: usize, r: usize, cl: usize, cr: usize| {
        let (ln, rn) = (l - cl, r - cr);
        if ln == 0 && rn == 0 {
            return;
        }
        if ln == rn && !blank_lines {
            items.push(Item::Op(ROp {
                kind: OpKind::Equal,
                l: cl,
                ln,
                r: cr,
                rn,
                inline: &[],
            }));
        } else {
            items.push(Item::Barrier);
        }
    };
    for h in own {
        let ((l, ln), (r, rn)) = (h.left(), h.right());
        gap(&mut items, l, r, cl, cr);
        items.extend(h.ops.iter().copied().map(Item::Op));
        cl = l + ln;
        cr = r + rn;
    }
    gap(&mut items, nl, nr, cl, cr);

    let is_change = |i: &Item<'_>| matches!(i, Item::Op(o) if o.kind.is_change());
    let changes: Vec<usize> = (0..items.len()).filter(|&i| is_change(&items[i])).collect();
    let Some(&first) = changes.first() else {
        return Vec::new();
    };
    let limit = 2 * u64::from(context);
    let equal_between = |a: usize, b: usize| -> u64 {
        items[a + 1..b]
            .iter()
            .map(|i| match i {
                Item::Barrier => u64::MAX,
                Item::Op(o) if o.kind == OpKind::Equal => o.ln as u64,
                Item::Op(_) => 0,
            })
            .fold(0u64, u64::saturating_add)
    };
    let mut groups = Vec::new();
    let mut group = (first, first);
    for &c in &changes[1..] {
        if equal_between(group.1, c) <= limit {
            group.1 = c;
        } else {
            groups.push(group);
            group = (c, c);
        }
    }
    groups.push(group);

    let context = usize::try_from(context).unwrap_or(usize::MAX);
    groups
        .into_iter()
        .map(|(a, b)| {
            let mut ops = Vec::new();
            // Leading context, nearest first.
            let mut need = context;
            let mut i = a;
            while need > 0 && i > 0 {
                i -= 1;
                match items[i] {
                    Item::Op(o) if o.kind == OpKind::Equal => {
                        let take = need.min(o.ln);
                        ops.push(ROp {
                            l: o.l + o.ln - take,
                            r: o.r + o.rn - take,
                            ln: take,
                            rn: take,
                            ..o
                        });
                        need -= take;
                    }
                    Item::Op(o) if o.kind == OpKind::Ignored => ops.push(o),
                    _ => break,
                }
            }
            ops.reverse();
            for item in &items[a..=b] {
                if let Item::Op(o) = item {
                    ops.push(*o);
                }
            }
            // Trailing context.
            let mut need = context;
            let mut i = b + 1;
            while need > 0 && i < items.len() {
                match items[i] {
                    Item::Op(o) if o.kind == OpKind::Equal => {
                        let take = need.min(o.ln);
                        ops.push(ROp {
                            ln: take,
                            rn: take,
                            ..o
                        });
                        need -= take;
                    }
                    Item::Op(o) if o.kind == OpKind::Ignored => ops.push(o),
                    _ => break,
                }
                i += 1;
            }
            RHunk { ops }
        })
        .collect()
}
