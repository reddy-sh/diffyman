//! Slider compaction of a line alignment (Diff Document spec §7.3.1).
//!
//! A run of changed lines is a **slider** when the same edit can be shown at
//! more than one position: the line just above the run equals its last line
//! (or the line just below equals its first), so the run can shift by one
//! line and still describe the same change. Inserting a function after
//! another one is the classic case: the blank line or closing brace between
//! the two can be reported at either end of the insertion. The line diff
//! algorithms place sliders wherever their search ends, so this pass moves
//! each one to a canonical position, the way git's diff does by default.
//!
//! The rules, for one side at a time (left first, then right, the right pass
//! seeing the left result):
//!
//! 1. **Slide.** Each run of changed lines is shifted up as far as it goes,
//!    then down as far as it goes. A run that touches another run of the same
//!    side while shifting merges with it, and a merged run is slid again,
//!    until its size stops changing. The highest and lowest positions are
//!    recorded, and so is the lowest position at which the other side has
//!    changed lines between the same two matched lines.
//! 2. **Place.** A run that cannot shift stays where it is. A run that can be
//!    lined up with changed lines of the other side goes to the lowest such
//!    position, so a deletion and an insertion next to each other form one
//!    change. Otherwise the indent heuristic below picks the position.
//!
//! The **indent heuristic** was designed and tuned for git by Michael
//! Haggerty (2016, the `--indent-heuristic` option, the default since git
//! 2.14) on a corpus of hand-rated sliders. It rates each candidate position
//! by the two boundaries the run would have, one above its first line and
//! one below its last line. A boundary is rated by the indentation of the
//! line right after it and of the nearest non-blank lines around it, and by
//! the blank lines next to it: a boundary next to a blank line, or just
//! before a line that is less indented than the code above it, is a natural
//! place to split code. The position whose two boundaries rate best wins;
//! ties go to the lower position. The weights below are the published ones.
//!
//! This module is an independent implementation from that description; it
//! shares no code with git. Indentation is measured on the original line
//! content (spaces count 1, a tab advances to the next multiple of 8, the
//! first other character ends it, capped at [`MAX_INDENT`]); a line with no
//! other character is blank. Only U+0020 and U+0009 count as whitespace here.
//!
//! Every shift swaps a changed line with an equal-key matched line of the
//! same side, so the result is still a valid alignment with the same number
//! of changed lines on each side, and the same multiset of changed keys.
//! Which lines form one change can differ, though: runs merge, and a run can
//! end up next to a change on the other side. So when blank-only changes are
//! ignored afterwards (`ignore.blankLines`, spec §7.3 step 3), a blank line
//! that joined a change with a non-blank line is counted where it was ignored
//! before (or the other way round), as with git's `--ignore-blank-lines`.
//!
//! **Work limit.** Sliding is linear in practice but has no useful upper
//! bound on adversarial input (a merged run slides again), so the pass counts
//! work units (one per shift or absorbed line, one per line comparison plus
//! one per 64 bytes compared outside the token range, one per line measured
//! by the heuristic) and stops for good once [`compaction_budget`] is spent.
//! What has been done stays; the rest of the alignment is left as the
//! algorithm returned it. The count does not depend on the platform.

use crate::line_diff::Alignment;

/// Indentation above this many columns counts as this many.
pub const MAX_INDENT: u32 = 200;

/// At most this many blank lines are counted on either side of a boundary.
pub const MAX_BLANKS: u32 = 20;

/// The heuristic looks at most this many positions above the lowest one.
pub const INDENT_HEURISTIC_MAX_SLIDING: usize = 100;

// Weights of the indent heuristic (see the module docs).
const START_OF_FILE_PENALTY: i64 = 1;
const END_OF_FILE_PENALTY: i64 = 21;
const TOTAL_BLANK_WEIGHT: i64 = -30;
const POST_BLANK_WEIGHT: i64 = 6;
const RELATIVE_INDENT_PENALTY: i64 = -4;
const RELATIVE_INDENT_WITH_BLANK_PENALTY: i64 = 10;
const RELATIVE_OUTDENT_PENALTY: i64 = 24;
const RELATIVE_OUTDENT_WITH_BLANK_PENALTY: i64 = 17;
const RELATIVE_DEDENT_PENALTY: i64 = 23;
const RELATIVE_DEDENT_WITH_BLANK_PENALTY: i64 = 17;
const INDENT_WEIGHT: i64 = 60;

/// Work units allowed per line (both sides counted).
pub const SLIDER_WORK_PER_LINE: u64 = 64;

/// Smallest budget, so small inputs never reach the limit.
pub const SLIDER_WORK_MIN: u64 = 1_000_000;

/// The work budget of [`compact`] for two sides of `left_lines` and
/// `right_lines` lines.
pub fn compaction_budget(left_lines: usize, right_lines: usize) -> u64 {
    let lines = (left_lines as u64).saturating_add(right_lines as u64);
    SLIDER_WORK_PER_LINE
        .saturating_mul(lines)
        .max(SLIDER_WORK_MIN)
}

/// One side of the comparison, as the slider needs it.
pub trait SliderSide {
    /// Number of lines.
    fn len(&self) -> usize;

    /// True when there are no lines.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True when lines `i` and `j` of this side have equal comparison keys,
    /// and the work units the comparison cost (at least 1).
    fn same(&mut self, i: usize, j: usize) -> (bool, u64);

    /// The content of line `i` (without its terminator), for indentation.
    fn content(&self, i: usize) -> &[u8];
}

/// A side given as a token per line, with line contents for the heuristic.
/// Each comparison costs one unit.
pub struct TokenSide<'a, C> {
    /// One token per line; equal tokens are equal keys.
    pub tokens: &'a [u32],
    /// Line contents by line number.
    pub content: C,
}

impl<'a, C: Fn(usize) -> &'a [u8]> SliderSide for TokenSide<'a, C> {
    fn len(&self) -> usize {
        self.tokens.len()
    }

    fn same(&mut self, i: usize, j: usize) -> (bool, u64) {
        (self.tokens[i] == self.tokens[j], 1)
    }

    fn content(&self, i: usize) -> &[u8] {
        (self.content)(i)
    }
}

/// What [`compact`] did. Not part of the Diff Document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompactReport {
    /// Work units spent.
    pub work: u64,
    /// True when the budget ran out before every run was placed.
    pub exhausted: bool,
}

/// Compacts `alignment` in place: left side first, then right. `left` and
/// `right` describe the sides' lines; their lengths must equal the flag
/// vectors'. An inconsistent alignment (unequal numbers of matched lines) is
/// left unchanged.
pub fn compact(
    alignment: &mut Alignment,
    left: &mut impl SliderSide,
    right: &mut impl SliderSide,
    budget: u64,
) -> CompactReport {
    let mut work = Work {
        spent: 0,
        budget,
        exhausted: false,
    };
    let ok = alignment.removed.len() == left.len()
        && alignment.added.len() == right.len()
        && alignment.is_consistent();
    if ok {
        let Alignment { removed, added } = alignment;
        compact_side(removed, added, left, &mut work);
        compact_side(added, removed, right, &mut work);
    }
    CompactReport {
        work: work.spent,
        exhausted: work.exhausted,
    }
}

struct Work {
    spent: u64,
    budget: u64,
    exhausted: bool,
}

impl Work {
    /// Spends `units`; false once the budget is gone (and from then on).
    fn spend(&mut self, units: u64) -> bool {
        if self.exhausted {
            return false;
        }
        self.spent = self.spent.saturating_add(units);
        if self.spent > self.budget {
            self.exhausted = true;
        }
        !self.exhausted
    }
}

/// A run of changed lines `start..end` on one side (empty runs sit between
/// two matched lines, or at a file edge).
#[derive(Debug, Clone, Copy)]
struct Run {
    start: usize,
    end: usize,
}

impl Run {
    fn is_empty(self) -> bool {
        self.start == self.end
    }

    fn len(self) -> usize {
        self.end - self.start
    }
}

/// The first run of `changed` (possibly empty).
fn first_run(changed: &[bool]) -> Run {
    let end = changed.iter().take_while(|c| **c).count();
    Run { start: 0, end }
}

/// The run after `run`: one matched line further down. `None` at the end.
fn next_run(changed: &[bool], run: Run) -> Option<Run> {
    if run.end >= changed.len() {
        return None;
    }
    let start = run.end + 1;
    let end = start + changed[start..].iter().take_while(|c| **c).count();
    Some(Run { start, end })
}

/// The run before `run`: one matched line further up. `None` at the start.
fn previous_run(changed: &[bool], run: Run) -> Option<Run> {
    if run.start == 0 {
        return None;
    }
    let end = run.start - 1;
    let start = end - changed[..end].iter().rev().take_while(|c| **c).count();
    Some(Run { start, end })
}

/// Shifts `run` up one line if the line above it equals its last line, and
/// absorbs a run it then touches. False when it cannot shift or the budget
/// is gone.
fn shift_up(
    changed: &mut [bool],
    run: &mut Run,
    side: &mut impl SliderSide,
    work: &mut Work,
) -> bool {
    if run.start == 0 || run.is_empty() {
        return false;
    }
    let (same, cost) = side.same(run.start - 1, run.end - 1);
    if !work.spend(cost) || !same {
        return false;
    }
    run.start -= 1;
    run.end -= 1;
    changed[run.start] = true;
    changed[run.end] = false;
    let absorbed = changed[..run.start]
        .iter()
        .rev()
        .take_while(|c| **c)
        .count();
    run.start -= absorbed;
    work.spend(1 + absorbed as u64);
    true
}

/// Shifts `run` down one line if the line below it equals its first line,
/// and absorbs a run it then touches.
fn shift_down(
    changed: &mut [bool],
    run: &mut Run,
    side: &mut impl SliderSide,
    work: &mut Work,
) -> bool {
    if run.end >= changed.len() || run.is_empty() {
        return false;
    }
    let (same, cost) = side.same(run.start, run.end);
    if !work.spend(cost) || !same {
        return false;
    }
    changed[run.start] = false;
    changed[run.end] = true;
    run.start += 1;
    run.end += 1;
    let absorbed = changed[run.end..].iter().take_while(|c| **c).count();
    run.end += absorbed;
    work.spend(1 + absorbed as u64);
    true
}

/// Compacts the runs of `changed`; `other` is the other side's flags, read
/// only. The k-th run of one side lies between the same two matched lines as
/// the k-th run of the other side, so the two walks advance together.
fn compact_side(changed: &mut [bool], other: &[bool], side: &mut impl SliderSide, work: &mut Work) {
    let mut run = first_run(changed);
    let mut partner = first_run(other);
    loop {
        if !run.is_empty() {
            place_run(changed, other, &mut run, &mut partner, side, work);
        }
        if work.exhausted {
            return;
        }
        match (next_run(changed, run), next_run(other, partner)) {
            (Some(r), Some(p)) => {
                run = r;
                partner = p;
            }
            _ => return,
        }
    }
}

/// Moves one partner run along with a shift of `run`.
fn follow(other: &[bool], partner: Run, up: bool) -> Run {
    let moved = if up {
        previous_run(other, partner)
    } else {
        next_run(other, partner)
    };
    // Both sides have the same number of matched lines, so the partner
    // always exists; keep the old one if it somehow does not.
    moved.unwrap_or(partner)
}

fn place_run(
    changed: &mut [bool],
    other: &[bool],
    run: &mut Run,
    partner: &mut Run,
    side: &mut impl SliderSide,
    work: &mut Work,
) {
    let mut highest_end;
    let mut aligned_end;
    loop {
        let size = run.len();
        aligned_end = None;
        while shift_up(changed, run, side, work) {
            *partner = follow(other, *partner, true);
        }
        highest_end = run.end;
        if !partner.is_empty() {
            aligned_end = Some(run.end);
        }
        while shift_down(changed, run, side, work) {
            *partner = follow(other, *partner, false);
            if !partner.is_empty() {
                aligned_end = Some(run.end);
            }
        }
        if work.exhausted || run.len() == size {
            break;
        }
    }
    if work.exhausted || run.end == highest_end {
        return;
    }
    let target = match aligned_end {
        Some(end) => end,
        None => best_position(side, *run, highest_end, work),
    };
    while run.end > target && !work.exhausted {
        if !shift_up(changed, run, side, work) {
            break;
        }
        *partner = follow(other, *partner, true);
    }
}

/// The end position (exclusive) the indent heuristic picks for `run`, which
/// sits at its lowest position and can go up to end at `highest_end`.
fn best_position(side: &impl SliderSide, run: Run, highest_end: usize, work: &mut Work) -> usize {
    let size = run.len();
    // Positions more than `size + 1` above the lowest one repeat the same
    // boundaries (the run then consists of repeated lines), and positions
    // more than the sliding cap above it are not considered.
    let lowest_considered = highest_end
        .max(run.end.saturating_sub(size + 1))
        .max(run.end.saturating_sub(INDENT_HEURISTIC_MAX_SLIDING));
    let mut best: Option<(usize, Score)> = None;
    for end in lowest_considered..=run.end {
        let mut score = Score::default();
        score.add(&measure(side, end, work));
        score.add(&measure(side, end - size, work));
        if best.is_none_or(|(_, b)| score.cmp(&b) <= 0) {
            best = Some((end, score));
        }
        if work.exhausted {
            return run.end;
        }
    }
    best.map_or(run.end, |(end, _)| end)
}

/// Indentation of a line in columns, or `None` for a blank line.
fn indent(content: &[u8]) -> Option<u32> {
    let mut columns = 0u32;
    for &b in content {
        match b {
            b' ' => columns += 1,
            b'\t' => columns += 8 - columns % 8,
            _ => return Some(columns),
        }
        if columns >= MAX_INDENT {
            return Some(MAX_INDENT);
        }
    }
    None
}

/// What the heuristic looks at around one boundary: the boundary sits just
/// above line `at` (`at == len` is the end of the file).
#[derive(Debug, Clone, Copy, Default)]
struct Boundary {
    end_of_file: bool,
    /// Indentation of line `at` (`None`: blank or end of file).
    indent: Option<u32>,
    /// Blank lines directly above the boundary (at most [`MAX_BLANKS`]).
    blanks_above: u32,
    /// Indentation of the nearest non-blank line above (`None`: none).
    indent_above: Option<u32>,
    /// Blank lines directly below line `at` (at most [`MAX_BLANKS`]).
    blanks_below: u32,
    /// Indentation of the nearest non-blank line below line `at`.
    indent_below: Option<u32>,
}

fn measure(side: &impl SliderSide, at: usize, work: &mut Work) -> Boundary {
    let len = side.len();
    let mut b = Boundary {
        end_of_file: at >= len,
        indent: if at < len {
            indent(side.content(at))
        } else {
            None
        },
        ..Boundary::default()
    };
    let mut lines = 1u64;
    for i in (0..at.min(len)).rev() {
        lines += 1;
        b.indent_above = indent(side.content(i));
        if b.indent_above.is_some() {
            break;
        }
        b.blanks_above += 1;
        if b.blanks_above == MAX_BLANKS {
            // A long blank stretch counts as the start of a new section.
            b.indent_above = Some(0);
            break;
        }
    }
    for i in at.saturating_add(1)..len {
        lines += 1;
        b.indent_below = indent(side.content(i));
        if b.indent_below.is_some() {
            break;
        }
        b.blanks_below += 1;
        if b.blanks_below == MAX_BLANKS {
            b.indent_below = Some(0);
            break;
        }
    }
    work.spend(lines);
    b
}

/// The combined rating of a position: lower is better.
#[derive(Debug, Clone, Copy, Default)]
struct Score {
    /// Sum of the boundaries' effective indentation (a blank or missing line
    /// counts as -1).
    indent: i64,
    penalty: i64,
}

impl Score {
    fn add(&mut self, b: &Boundary) {
        if b.indent_above.is_none() && b.blanks_above == 0 {
            self.penalty += START_OF_FILE_PENALTY;
        }
        if b.end_of_file {
            self.penalty += END_OF_FILE_PENALTY;
        }
        // Blank lines just below the boundary: line `at` itself and the ones
        // after it, when line `at` is blank.
        let blanks_after = if b.indent.is_none() {
            1 + i64::from(b.blanks_below)
        } else {
            0
        };
        let blanks = i64::from(b.blanks_above) + blanks_after;
        self.penalty += TOTAL_BLANK_WEIGHT * blanks + POST_BLANK_WEIGHT * blanks_after;
        let effective = b.indent.or(b.indent_below);
        self.indent += effective.map_or(-1, i64::from);
        let (Some(here), Some(above)) = (effective, b.indent_above) else {
            return;
        };
        let with_blank = blanks != 0;
        self.penalty += if here > above {
            if with_blank {
                RELATIVE_INDENT_WITH_BLANK_PENALTY
            } else {
                RELATIVE_INDENT_PENALTY
            }
        } else if here == above {
            0
        } else if b.indent_below.is_some_and(|below| below > here) {
            // Less indented than above but more indented below: the boundary
            // sits on an outdented line inside a block.
            if with_blank {
                RELATIVE_OUTDENT_WITH_BLANK_PENALTY
            } else {
                RELATIVE_OUTDENT_PENALTY
            }
        } else if with_blank {
            RELATIVE_DEDENT_WITH_BLANK_PENALTY
        } else {
            RELATIVE_DEDENT_PENALTY
        };
    }

    /// Negative when `self` is better than `other`, 0 when equally good.
    fn cmp(&self, other: &Score) -> i64 {
        let by_indent = match self.indent.cmp(&other.indent) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        INDENT_WEIGHT * by_indent + (self.penalty - other.penalty)
    }
}

#[cfg(test)]
mod tests;
