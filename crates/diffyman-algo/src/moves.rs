//! Moved-block detection (`docs/spec/text-granularity-and-moves.md` §4),
//! algorithm ID `text.moves@1`.
//!
//! Detection is a post-pass over the full op list. It never changes an op; it
//! only pairs blocks of deleted left lines with blocks of inserted right lines
//! that sit in different ops.
//!
//! 1. **Pure moves** (spec §4.3). Seeds come from a key index: a pair of a
//!    deleted and an inserted line with equal move keys, where the key occurs
//!    at most [`MOVE_MAX_KEY_OCCURRENCES`] times among the deleted lines and
//!    among the inserted lines. Each seed is extended up and down along its
//!    diagonal to a maximal run (every run is walked once), split where the
//!    indentation delta changes, and trimmed of blank edge lines. Runs are
//!    then selected greedily (alnum count, length, left start, right start);
//!    a run that loses lines to an earlier selection is split into its
//!    unclaimed sub-runs, which compete again.
//! 2. **Joining** (spec §4.3 step 4). Each pure move walks down by the
//!    edited edge extension inside its two ops and absorbs the next pure
//!    move of those ops when it reaches it, so a moved block with an edited
//!    line inside is one edited move instead of two pure pieces.
//! 3. **Edited moves** (spec §4.5), over the lines that are still unclaimed.
//!    Line similarity is a grapheme Levenshtein distance (bit-parallel, at most
//!    [`MOVE_EDIT_MAX_LINE_GRAPHEMES`] graphemes per line; longer lines pair
//!    only when their keys are equal), behind a length and histogram filter.
//!    For each pair of unclaimed segments in different ops that holds a seed
//!    (a line pair with similarity >= [`MOVE_EDIT_SEED_BP`]), a forward and a
//!    backward local alignment give, for every seed, the best-scoring block
//!    through it. The score is
//!    `sum(2*sim - 2*T) - T*(unpaired non-trivial lines)` with
//!    `T = MOVE_EDITED_MIN_SIM_BP`, which is `>= 0` exactly when the block
//!    similarity is `>= T`. Each block then grows at its edges by nearby
//!    pairs while its similarity stays `>= T` (the score alone would stop
//!    early, for example before a dropped comment line). Blocks are selected
//!    like pure runs. A candidate that keeps all its lines stays the best
//!    block through its seed (the segments only shrank around it); one that
//!    loses lines is rebuilt from its seed over the current segments, with
//!    the sweeps cached per segment pair. So a selection costs work only
//!    where it changed something.
//!
//! Work limits, all deterministic:
//!
//! - More than [`MOVE_MAX_CANDIDATE_LINES`] deleted or inserted lines: no
//!   detection at all (`skipped_limit`, no moves).
//! - The edited pass compares at most [`MOVE_EDIT_MAX_PAIRS`] line pairs,
//!   taking segment pairs smallest first; the ones that do not fit are not
//!   searched (`skipped_limit`, the other moves are kept). With more than
//!   that many segment pairs it is not run at all.
//! - The edited pass computes at most [`MOVE_EDIT_MAX_CELLS`] alignment cells
//!   plus traceback and extension steps in total. When the budget runs out,
//!   no further segment pair is swept, candidates that need a rebuild are
//!   dropped (`skipped_limit`) and the blocks already found are kept.
//!
//! The pure pass does work proportional to the number of seed pairs plus the
//! total length of the runs found; the per-key cap keeps the seed count at
//! most `MOVE_MAX_KEY_OCCURRENCES` times the number of deleted lines.
//!
//! Measured on one machine (release build): 1 million near-identical line
//! pairs take about 0.3 s, and the worst case for the filter (1 million pairs
//! of 248-grapheme lines that are permutations of each other, so every pair
//! runs the distance) about 1.5 s.

// Index arithmetic over two sides (l, r, i, j, a, b, k) reads better with the
// spec's short names.
#![allow(clippy::many_single_char_names)]

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use icu_casemap::{CaseMapper, CaseMapperBorrowed};
use unicode_segmentation::UnicodeSegmentation;

use crate::key::{self, WhitespaceMode};
use crate::ops::LineOp;

/// Algorithm ID of moved-block detection.
pub const MOVES_ID: &str = "text.moves@1";
/// Lines with at most this many alphanumeric code points are trivial.
pub const TRIVIAL_LINE_MAX_ALNUM: usize = 2;
/// Minimum alphanumeric code points per side of a move.
pub const MOVE_MIN_ALNUM: usize = 20;
/// Minimum non-trivial lines per side (unless one long line qualifies).
pub const MOVE_MIN_NONTRIVIAL_LINES: usize = 2;
/// Alphanumeric code points that let a single non-trivial line qualify.
pub const MOVE_SINGLE_LINE_MIN_ALNUM: usize = 40;
/// A key that occurs more often among the deleted (or inserted) lines cannot seed a pure run.
pub const MOVE_MAX_KEY_OCCURRENCES: usize = 64;
/// Detection is skipped when more lines than this are deleted or inserted.
pub const MOVE_MAX_CANDIDATE_LINES: usize = 50_000;
/// Minimum block similarity of an edited move, in basis points.
pub const MOVE_EDITED_MIN_SIM_BP: u32 = 8_000;
/// Minimum similarity of a paired line inside an edited move, in basis points.
pub const MOVE_LINE_PAIR_MIN_BP: u32 = 6_000;
/// Minimum similarity of the seed pair of an edited move, in basis points.
pub const MOVE_EDIT_SEED_BP: u32 = 9_000;
/// Longer lines (in graphemes of their move key) pair only when their keys are equal.
pub const MOVE_EDIT_MAX_LINE_GRAPHEMES: usize = 256;
/// Maximum line pairs (and segment pairs) the edited pass compares.
pub const MOVE_EDIT_MAX_PAIRS: u64 = 1_000_000;
/// Maximum alignment cells (plus traceback and extension steps) the edited pass computes.
pub const MOVE_EDIT_MAX_CELLS: u64 = 8_000_000;
/// An edge-extension step of an edited block may skip at most this many lines per side.
pub const MOVE_EDIT_EXTEND_SKIP: usize = 2;

const NO_OP: u32 = u32::MAX;
const NO_KEY: u32 = u32::MAX;
const FULL_BP: u32 = 10_000;
const WORDS: usize = MOVE_EDIT_MAX_LINE_GRAPHEMES.div_ceil(64);

/// Move kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    /// The lines match pairwise under the move key.
    Pure,
    /// The lines match with small edits.
    Edited,
}

/// A detected move over 0-based line indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetectedMove {
    /// First left line.
    pub left_start: usize,
    /// Number of left lines.
    pub left_len: usize,
    /// First right line.
    pub right_start: usize,
    /// Number of right lines.
    pub right_len: usize,
    /// Pure or edited.
    pub kind: MoveKind,
    /// Block similarity in basis points; 10000 for a pure move.
    pub similarity_bp: u32,
}

/// The comparison options that shape the move key (spec §4.4).
#[derive(Debug, Clone, Copy, Default)]
pub struct MoveOptions {
    /// The document's `ignore.whitespace`.
    pub whitespace: WhitespaceMode,
    /// The document's `ignore.case`.
    pub ignore_case: bool,
}

/// Result of [`detect_moves`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Moves {
    /// Moves sorted by left start, then right start.
    pub moves: Vec<DetectedMove>,
    /// A work limit engaged (`moves-skipped-limit`).
    pub skipped_limit: bool,
}

/// Detects moved blocks among deleted and inserted lines.
///
/// `left` and `right` are the line contents without terminators, and `ops`
/// is the full op list (it tiles both files).
pub fn detect_moves(left: &[&str], right: &[&str], ops: &[LineOp], options: &MoveOptions) -> Moves {
    let deleted: usize = ops
        .iter()
        .filter(|o| o.kind.is_change())
        .map(|o| o.left_len)
        .sum();
    let inserted: usize = ops
        .iter()
        .filter(|o| o.kind.is_change())
        .map(|o| o.right_len)
        .sum();
    if deleted > MOVE_MAX_CANDIDATE_LINES || inserted > MOVE_MAX_CANDIDATE_LINES {
        return Moves {
            moves: Vec::new(),
            skipped_limit: true,
        };
    }
    let mut keys = Keys::new(*options);
    let mut l = Side::build(left, ops, true, &mut keys);
    let mut r = Side::build(right, ops, false, &mut keys);
    let check_delta = matches!(
        options.whitespace,
        WhitespaceMode::None | WhitespaceMode::Trailing
    );
    let mut moves = Vec::new();
    pure_pass(&mut l, &mut r, keys.texts.len(), check_delta, &mut moves);
    join_pure(&mut l, &mut r, &keys.texts, &mut moves);
    let skipped_limit = edited_pass(&mut l, &mut r, &keys.texts, check_delta, &mut moves);
    moves.sort_by_key(|m| (m.left_start, m.right_start));
    Moves {
        moves,
        skipped_limit,
    }
}

// ---------------------------------------------------------------------------
// Line data
// ---------------------------------------------------------------------------

/// Interns move keys (spec §4.4) to dense ids, in first-seen order.
struct Keys {
    whitespace: WhitespaceMode,
    fold: Option<CaseMapperBorrowed<'static>>,
    ids: HashMap<String, u32>,
    texts: Vec<String>,
}

impl Keys {
    fn new(options: MoveOptions) -> Keys {
        Keys {
            whitespace: options.whitespace,
            fold: options.ignore_case.then(CaseMapper::new),
            ids: HashMap::new(),
            texts: Vec::new(),
        }
    }

    fn intern(&mut self, content: &str) -> u32 {
        let rest = content.trim_start_matches(key::is_whitespace);
        let normalized = key::normalize_whitespace(rest, self.whitespace);
        let text = match self.fold {
            Some(fold) => normalized.chars().map(|c| fold.simple_fold(c)).collect(),
            None => normalized.into_owned(),
        };
        if let Some(id) = self.ids.get(&text) {
            return *id;
        }
        let id = u32::try_from(self.texts.len()).unwrap_or(NO_KEY - 1);
        self.texts.push(text.clone());
        self.ids.insert(text, id);
        id
    }
}

/// Per-line data of one side. Only changed lines get a key and an op.
struct Side<'a> {
    /// Index of the change op holding the line, or `NO_OP`.
    op: Vec<u32>,
    key: Vec<u32>,
    indent: Vec<&'a str>,
    alnum: Vec<u32>,
    blank: Vec<bool>,
    claimed: Vec<bool>,
}

impl<'a> Side<'a> {
    fn build(lines: &[&'a str], ops: &[LineOp], left: bool, keys: &mut Keys) -> Side<'a> {
        let n = lines.len();
        let mut side = Side {
            op: vec![NO_OP; n],
            key: vec![NO_KEY; n],
            indent: vec![""; n],
            alnum: vec![0; n],
            blank: vec![false; n],
            claimed: vec![false; n],
        };
        for (index, op) in ops.iter().enumerate() {
            if !op.kind.is_change() {
                continue;
            }
            let (start, len) = if left {
                (op.left_start, op.left_len)
            } else {
                (op.right_start, op.right_len)
            };
            let id = u32::try_from(index).unwrap_or(NO_OP - 1);
            for i in start..(start + len).min(n) {
                side.op[i] = id;
            }
        }
        for (i, content) in lines.iter().enumerate() {
            if side.op[i] == NO_OP {
                continue;
            }
            let rest = content.trim_start_matches(key::is_whitespace);
            side.indent[i] = &content[..content.len() - rest.len()];
            side.alnum[i] = u32::try_from(content.chars().filter(|c| c.is_alphanumeric()).count())
                .unwrap_or(u32::MAX);
            side.blank[i] = rest.is_empty();
            side.key[i] = keys.intern(content);
        }
        side
    }

    fn len(&self) -> usize {
        self.op.len()
    }

    fn trivial(&self, i: usize) -> bool {
        self.alnum[i] as usize <= TRIVIAL_LINE_MAX_ALNUM
    }

    /// The minimum size of spec §4.2 for lines `start..start+len`.
    fn qualifies(&self, start: usize, len: usize) -> bool {
        let mut alnum = 0usize;
        let mut nontrivial = 0usize;
        let mut single = 0usize;
        for i in start..start + len {
            let a = self.alnum[i] as usize;
            alnum += a;
            if a > TRIVIAL_LINE_MAX_ALNUM {
                nontrivial += 1;
                single = a;
            }
        }
        alnum >= MOVE_MIN_ALNUM
            && (nontrivial >= MOVE_MIN_NONTRIVIAL_LINES
                || (nontrivial == 1 && single >= MOVE_SINGLE_LINE_MIN_ALNUM))
    }

    fn alnum_sum(&self, start: usize, len: usize) -> u64 {
        self.alnum[start..start + len]
            .iter()
            .map(|a| u64::from(*a))
            .sum()
    }

    fn claim(&mut self, start: usize, len: usize) {
        for c in &mut self.claimed[start..start + len] {
            *c = true;
        }
    }
}

/// The indentation delta of spec §4.3 step 3: a fixed string added to or
/// removed from the start of the left indent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delta<'a> {
    Add(&'a str),
    Remove(&'a str),
}

fn delta<'a>(left: &'a str, right: &'a str) -> Option<Delta<'a>> {
    if let Some(added) = right.strip_suffix(left) {
        Some(Delta::Add(added))
    } else {
        left.strip_suffix(right).map(Delta::Remove)
    }
}

/// True when lines `l..l+len` and `r..r+len` are a pure block: equal keys
/// pairwise and, if required, one indentation delta for every non-blank line.
fn is_pure(
    ls: &Side<'_>,
    rs: &Side<'_>,
    l: usize,
    r: usize,
    len: usize,
    check_delta: bool,
) -> bool {
    let mut current: Option<Delta<'_>> = None;
    for t in 0..len {
        let (a, b) = (l + t, r + t);
        if ls.key[a] != rs.key[b] {
            return false;
        }
        if !check_delta || ls.blank[a] {
            continue;
        }
        match (delta(ls.indent[a], rs.indent[b]), current) {
            (None, _) => return false,
            (Some(d), None) => current = Some(d),
            (Some(d), Some(c)) if d != c => return false,
            _ => {}
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Pure moves (spec §4.3)
// ---------------------------------------------------------------------------

/// A pure candidate run: left `l..l+k`, right `r..r+k`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    l: usize,
    r: usize,
    k: usize,
    alnum: u64,
}

impl Ord for Run {
    /// Greater is selected first: alnum count, then length, then smaller
    /// left start, then smaller right start.
    fn cmp(&self, other: &Run) -> Ordering {
        self.alnum
            .cmp(&other.alnum)
            .then(self.k.cmp(&other.k))
            .then(other.l.cmp(&self.l))
            .then(other.r.cmp(&self.r))
    }
}

impl PartialOrd for Run {
    fn partial_cmp(&self, other: &Run) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Trims blank pairs from both ends of a run with equal keys.
fn trimmed(ls: &Side<'_>, mut l: usize, mut r: usize, mut k: usize) -> Option<Run> {
    while k > 0 && ls.blank[l] {
        l += 1;
        r += 1;
        k -= 1;
    }
    while k > 0 && ls.blank[l + k - 1] {
        k -= 1;
    }
    (k > 0).then(|| Run {
        l,
        r,
        k,
        alnum: ls.alnum_sum(l, k),
    })
}

/// Splits a key run where the indentation delta changes. A line pair with no
/// delta (neither indent ends with the other) ends the sub-run and starts a
/// new one after it. Blank pairs between two deltas belong to both sub-runs
/// (they are trimmed later).
fn split_by_delta(
    ls: &Side<'_>,
    rs: &Side<'_>,
    l: usize,
    r: usize,
    k: usize,
    check_delta: bool,
    out: &mut Vec<Run>,
) {
    let mut emit = |from: usize, to: usize| {
        if to > from
            && let Some(run) = trimmed(ls, l + from, r + from, to - from)
        {
            out.push(run);
        }
    };
    if !check_delta {
        emit(0, k);
        return;
    }
    let mut start = 0;
    let mut current: Option<Delta<'_>> = None;
    let mut after_last = 0;
    for t in 0..k {
        let (a, b) = (l + t, r + t);
        if ls.blank[a] {
            continue;
        }
        match delta(ls.indent[a], rs.indent[b]) {
            None => {
                emit(start, t);
                start = t + 1;
                current = None;
            }
            Some(d) => match current {
                None => current = Some(d),
                Some(c) if c == d => {}
                Some(_) => {
                    emit(start, t);
                    start = after_last;
                    current = Some(d);
                }
            },
        }
        after_last = t + 1;
    }
    emit(start, k);
}

fn pure_pass(
    ls: &mut Side<'_>,
    rs: &mut Side<'_>,
    num_keys: usize,
    check_delta: bool,
    out: &mut Vec<DetectedMove>,
) {
    let (nl, nr) = (ls.len(), rs.len());
    let mut deleted = vec![0usize; num_keys];
    let mut inserted = vec![0usize; num_keys];
    let mut right_by_key: Vec<Vec<usize>> = vec![Vec::new(); num_keys];
    for i in 0..nl {
        if ls.op[i] != NO_OP {
            deleted[ls.key[i] as usize] += 1;
        }
    }
    for j in 0..nr {
        if rs.op[j] != NO_OP {
            inserted[rs.key[j] as usize] += 1;
            right_by_key[rs.key[j] as usize].push(j);
        }
    }

    let mut runs = Vec::new();
    // End (exclusive left index) of the last run found on each diagonal.
    // Seeds are visited in left order, so a seed below that end is covered.
    let mut diagonal_end: HashMap<usize, usize> = HashMap::new();
    for i in 0..nl {
        let op_l = ls.op[i];
        if op_l == NO_OP {
            continue;
        }
        let k = ls.key[i] as usize;
        if deleted[k] > MOVE_MAX_KEY_OCCURRENCES || inserted[k] > MOVE_MAX_KEY_OCCURRENCES {
            continue;
        }
        for &j in &right_by_key[k] {
            let op_r = rs.op[j];
            if op_r == op_l {
                continue; // spec §4.1 rule 4: same op is an in-place rewrite
            }
            let diagonal = j + nl - i;
            if diagonal_end.get(&diagonal).is_some_and(|end| *end > i) {
                continue;
            }
            let fits =
                |a: usize, b: usize| ls.op[a] == op_l && rs.op[b] == op_r && ls.key[a] == rs.key[b];
            let (mut si, mut sj) = (i, j);
            while si > 0 && sj > 0 && fits(si - 1, sj - 1) {
                si -= 1;
                sj -= 1;
            }
            let (mut ei, mut ej) = (i + 1, j + 1);
            while ei < nl && ej < nr && fits(ei, ej) {
                ei += 1;
                ej += 1;
            }
            diagonal_end.insert(diagonal, ei);
            split_by_delta(ls, rs, si, sj, ei - si, check_delta, &mut runs);
        }
    }

    let mut heap: BinaryHeap<Run> = runs.into();
    while let Some(run) = heap.pop() {
        let taken = |t: usize| ls.claimed[run.l + t] || rs.claimed[run.r + t];
        if (0..run.k).any(taken) {
            let mut t = 0;
            while t < run.k {
                if taken(t) {
                    t += 1;
                    continue;
                }
                let s = t;
                while t < run.k && !taken(t) {
                    t += 1;
                }
                if let Some(sub) = trimmed(ls, run.l + s, run.r + s, t - s) {
                    heap.push(sub);
                }
            }
            continue;
        }
        // A run that fails the minimum size has no sub-run that passes it.
        if ls.qualifies(run.l, run.k) && rs.qualifies(run.r, run.k) {
            ls.claim(run.l, run.k);
            rs.claim(run.r, run.k);
            out.push(DetectedMove {
                left_start: run.l,
                left_len: run.k,
                right_start: run.r,
                right_len: run.k,
                kind: MoveKind::Pure,
                similarity_bp: FULL_BP,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Joining pure pieces of an edited move (spec §4.3 step 4)
// ---------------------------------------------------------------------------

/// One walk step below a block: the new ends and (sum, n), plus the pure
/// move it absorbs, if any.
#[derive(Debug, Clone, Copy)]
struct Step {
    l1: usize,
    r1: usize,
    sum: u64,
    n: u64,
    skip: (usize, usize),
    absorbs: Option<usize>,
}

/// Joins pure moves that are pieces of one edited move.
///
/// Each pure move, in left-start order, walks down by the edge extension of
/// spec §4.5 step 2 inside its two ops: a step adds a pairable pair of
/// unclaimed lines, or the next pure move of the same two ops when it starts
/// there, skipping at most [`MOVE_EDIT_EXTEND_SKIP`] unclaimed lines per
/// side, while the block similarity stays at or above
/// [`MOVE_EDITED_MIN_SIM_BP`]. The block ends at the last pure move it
/// absorbed whose join is a real edit (the joined lines are not all equal
/// keys pairwise; pieces split only by the indentation rule stay apart). A
/// move that absorbs none is left exactly as it is.
fn join_pure(
    ls: &mut Side<'_>,
    rs: &mut Side<'_>,
    key_texts: &[String],
    moves: &mut Vec<DetectedMove>,
) {
    if moves.len() < 2 {
        return;
    }
    moves.sort_by_key(|m| (m.left_start, m.right_start));
    let by_start: HashMap<(usize, usize), usize> = moves
        .iter()
        .enumerate()
        .map(|(k, m)| ((m.left_start, m.right_start), k))
        .collect();
    let mut open = vec![true; moves.len()];
    let mut sim = LineSim::new(key_texts);
    let mut out = Vec::with_capacity(moves.len());
    let (nl, nr) = (ls.len(), rs.len());
    let nontrivial = |side: &Side<'_>, from: usize, to: usize| {
        (from..to).filter(|i| !side.trivial(*i)).count() as u64
    };
    for k in 0..moves.len() {
        if !open[k] {
            continue;
        }
        open[k] = false;
        let m = moves[k];
        let (op_l, op_r) = (ls.op[m.left_start], rs.op[m.right_start]);
        let free_l = |i: usize| ls.op[i] == op_l && !ls.claimed[i];
        let free_r = |j: usize| rs.op[j] == op_r && !rs.claimed[j];
        let (l0, r0) = (m.left_start, m.right_start);
        let (mut l1, mut r1) = (l0 + m.left_len, r0 + m.right_len);
        let core = nontrivial(ls, l0, l1);
        let (mut sum, mut n) = (u64::from(FULL_BP) * core, 2 * core);
        // The block as of the last accepted join: (l1, r1, sum, n, absorbed).
        let mut joined: Option<(usize, usize, u64, u64, Vec<usize>)> = None;
        let mut absorbed = Vec::new();
        loop {
            let mut best: Option<Step> = None;
            for skip_l in 0..=MOVE_EDIT_EXTEND_SKIP {
                for skip_r in 0..=MOVE_EDIT_EXTEND_SKIP {
                    let (i, j) = (l1 + skip_l, r1 + skip_r);
                    if i >= nl || j >= nr || !(l1..i).all(free_l) || !(r1..j).all(free_r) {
                        continue;
                    }
                    let skipped = nontrivial(ls, l1, i) + nontrivial(rs, r1, j);
                    let step = if free_l(i) && free_r(j) {
                        if ls.trivial(i) || rs.trivial(j) {
                            continue;
                        }
                        let Some(s) = sim.pairable(ls.key[i], rs.key[j]) else {
                            continue;
                        };
                        Step {
                            l1: i + 1,
                            r1: j + 1,
                            sum: sum + u64::from(s),
                            n: n + skipped + 2,
                            skip: (skip_l, skip_r),
                            absorbs: None,
                        }
                    } else if ls.op[i] == op_l
                        && rs.op[j] == op_r
                        && let Some(&k2) = by_start.get(&(i, j))
                        && open[k2]
                    {
                        let next = moves[k2];
                        let inner = nontrivial(ls, i, i + next.left_len);
                        Step {
                            l1: i + next.left_len,
                            r1: j + next.right_len,
                            sum: sum + u64::from(FULL_BP) * inner,
                            n: n + skipped + 2 * inner,
                            skip: (skip_l, skip_r),
                            absorbs: Some(k2),
                        }
                    } else {
                        continue;
                    };
                    if 2 * step.sum < u64::from(MOVE_EDITED_MIN_SIM_BP) * step.n {
                        continue;
                    }
                    // Highest block similarity, then fewer skipped lines,
                    // then fewer skipped left lines.
                    let better = best.is_none_or(|b| {
                        let lhs = u128::from(step.sum) * u128::from(b.n);
                        let rhs = u128::from(b.sum) * u128::from(step.n);
                        lhs > rhs
                            || (lhs == rhs
                                && (step.skip.0 + step.skip.1, step.skip.0)
                                    < (b.skip.0 + b.skip.1, b.skip.0))
                    });
                    if better {
                        best = Some(step);
                    }
                }
            }
            let Some(step) = best else { break };
            (l1, r1, sum, n) = (step.l1, step.r1, step.sum, step.n);
            if let Some(k2) = step.absorbs {
                absorbed.push(k2);
                let equal =
                    l1 - l0 == r1 - r0 && (0..l1 - l0).all(|t| ls.key[l0 + t] == rs.key[r0 + t]);
                if !equal {
                    joined = Some((l1, r1, sum, n, absorbed.clone()));
                }
            }
        }
        let Some((l1, r1, sum, n, absorbed)) = joined else {
            out.push(m);
            continue;
        };
        for k2 in absorbed {
            open[k2] = false;
        }
        ls.claim(l0, l1 - l0);
        rs.claim(r0, r1 - r0);
        let similarity = (2 * sum / n.max(1)).min(u64::from(FULL_BP));
        out.push(DetectedMove {
            left_start: l0,
            left_len: l1 - l0,
            right_start: r0,
            right_len: r1 - r0,
            kind: MoveKind::Edited,
            similarity_bp: u32::try_from(similarity).unwrap_or(FULL_BP),
        });
    }
    *moves = out;
}

// ---------------------------------------------------------------------------
// Line similarity (spec §4.5)
// ---------------------------------------------------------------------------

/// Graphemes of one line's move key, as dense ids, with a 64-bucket histogram.
struct Graphemes {
    ids: Vec<u32>,
    histogram: [u16; 64],
    long: bool,
}

/// The graphemes of a move key, with ids from `ids` (new graphemes get the next id).
fn graphemes_of<'t>(text: &'t str, ids: &mut HashMap<&'t str, u32>) -> Graphemes {
    let mut g = Graphemes {
        ids: Vec::new(),
        histogram: [0; 64],
        long: false,
    };
    for grapheme in text.graphemes(true) {
        let next = u32::try_from(ids.len()).unwrap_or(u32::MAX);
        let id = *ids.entry(grapheme).or_insert(next);
        g.ids.push(id);
        let bucket = &mut g.histogram[(id % 64) as usize];
        *bucket = bucket.saturating_add(1);
    }
    if g.ids.len() > MOVE_EDIT_MAX_LINE_GRAPHEMES {
        g.long = true;
        g.ids = Vec::new();
    }
    g
}

/// Line similarity of spec §4.5 computed on demand, for the few pairs next
/// to a pure move (spec §4.3 step 4). Graphemes are cached per move key.
struct LineSim<'k> {
    key_texts: &'k [String],
    ids: HashMap<&'k str, u32>,
    graphemes: HashMap<u32, Graphemes>,
}

impl<'k> LineSim<'k> {
    fn new(key_texts: &'k [String]) -> LineSim<'k> {
        LineSim {
            key_texts,
            ids: HashMap::new(),
            graphemes: HashMap::new(),
        }
    }

    fn ensure(&mut self, key: u32) {
        if !self.graphemes.contains_key(&key) {
            let g = graphemes_of(self.key_texts[key as usize].as_str(), &mut self.ids);
            self.graphemes.insert(key, g);
        }
    }

    /// The similarity of two move keys if the pair is pairable.
    fn pairable(&mut self, a: u32, b: u32) -> Option<u32> {
        if a == b {
            return Some(FULL_BP);
        }
        self.ensure(a);
        self.ensure(b);
        let (ga, gb) = (&self.graphemes[&a], &self.graphemes[&b]);
        if ga.long || gb.long {
            return None;
        }
        // Pattern masks over a compact alphabet: the distinct graphemes of `a`,
        // plus one all-zero row for graphemes that do not occur in `a`.
        let mut local: HashMap<u32, usize> = HashMap::new();
        for &g in &ga.ids {
            let next = local.len();
            local.entry(g).or_insert(next);
        }
        let absent = local.len();
        let mut peq = vec![0u64; (absent + 1) * WORDS];
        for (p, g) in ga.ids.iter().enumerate() {
            peq[local[g] * WORDS + p / 64] |= 1 << (p % 64);
        }
        let text = Graphemes {
            ids: gb
                .ids
                .iter()
                .map(|g| u32::try_from(local.get(g).copied().unwrap_or(absent)).unwrap_or(0))
                .collect(),
            histogram: gb.histogram,
            long: false,
        };
        pair_similarity(ga, &text, &peq)
    }
}

/// `floor(10000 * (m - d) / m)` if it is at least [`MOVE_LINE_PAIR_MIN_BP`].
fn pair_similarity(a: &Graphemes, b: &Graphemes, peq: &[u64]) -> Option<u32> {
    if a.long || b.long {
        return None;
    }
    let (la, lb) = (a.ids.len(), b.ids.len());
    let m = la.max(lb);
    if m == 0 {
        return Some(FULL_BP);
    }
    // sim >= 6000 bp  <=>  d <= 0.4 m.
    let max_d = 2 * m / 5;
    if la.abs_diff(lb) > max_d {
        return None;
    }
    // Each edit changes the bucket histogram's L1 distance by at most 2.
    let l1: usize = a
        .histogram
        .iter()
        .zip(&b.histogram)
        .map(|(x, y)| usize::from(x.abs_diff(*y)))
        .sum();
    if l1.div_ceil(2) > max_d {
        return None;
    }
    let d = if la == 0 {
        lb
    } else {
        myers_distance(peq, la, &b.ids)
    };
    if d > max_d {
        return None;
    }
    u32::try_from(10_000 * (m - d) / m).ok()
}

/// Levenshtein distance between a pattern of `m` graphemes (1..=256), whose
/// match masks are in `peq` (`WORDS` words per grapheme id), and `text`.
/// Myers' bit-parallel algorithm in Hyyrö's multi-word form.
fn myers_distance(peq: &[u64], m: usize, text: &[u32]) -> usize {
    let words = m.div_ceil(64);
    let last_bit = 1u64 << ((m - 1) % 64);
    let mut pv = [u64::MAX; WORDS];
    let mut mv = [0u64; WORDS];
    let mut score = m;
    for &g in text {
        let base = g as usize * WORDS;
        // The top row is D[0][j] = j, so every column starts with a +1 delta.
        let mut hin: i8 = 1;
        for w in 0..words {
            let mut eq = peq[base + w];
            let (p, n) = (pv[w], mv[w]);
            let xv = eq | n;
            if hin < 0 {
                eq |= 1;
            }
            let xh = ((eq & p).wrapping_add(p) ^ p) | eq;
            let mut ph = n | !(xh | p);
            let mut mh = p & xh;
            let high = if w + 1 == words { last_bit } else { 1 << 63 };
            let hout: i8 = if ph & high != 0 {
                1
            } else if mh & high != 0 {
                -1
            } else {
                0
            };
            ph <<= 1;
            mh <<= 1;
            if hin < 0 {
                mh |= 1;
            } else if hin > 0 {
                ph |= 1;
            }
            pv[w] = mh | !(xv | ph);
            mv[w] = ph & xv;
            hin = hout;
        }
        match hin {
            1 => score += 1,
            -1 => score -= 1,
            _ => {}
        }
    }
    score
}

// ---------------------------------------------------------------------------
// Edited moves (spec §4.5)
// ---------------------------------------------------------------------------

/// A maximal run of unclaimed lines of one change op on one side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seg {
    start: usize,
    end: usize,
    op: u32,
}

/// The segments of a side that hold at least one non-trivial line.
fn segments(side: &Side<'_>) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < side.len() {
        if side.op[i] == NO_OP || side.claimed[i] {
            i += 1;
            continue;
        }
        let (start, op) = (i, side.op[i]);
        let mut nontrivial = false;
        while i < side.len() && side.op[i] == op && !side.claimed[i] {
            nontrivial |= !side.trivial(i);
            i += 1;
        }
        if nontrivial {
            out.push(Seg { start, end: i, op });
        }
    }
    out
}

/// The segment holding the unclaimed changed line `i`.
fn segment_of(side: &Side<'_>, i: usize) -> Seg {
    let op = side.op[i];
    let fits = |k: usize| side.op[k] == op && !side.claimed[k];
    let mut start = i;
    while start > 0 && fits(start - 1) {
        start -= 1;
    }
    let mut end = i + 1;
    while end < side.len() && fits(end) {
        end += 1;
    }
    Seg { start, end, op }
}

/// An edited candidate block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Block {
    /// Alnum count of the paired left lines.
    alnum: u64,
    /// Number of paired lines.
    pairs: u64,
    l0: usize,
    l1: usize,
    r0: usize,
    r1: usize,
    /// The seed pair (left line, right line) the block was grown from.
    seed: (usize, usize),
    kind: MoveKind,
    similarity_bp: u32,
}

impl Ord for Block {
    /// Greater is selected first: alnum count of paired lines, then number of
    /// pairs, then smaller left start, then smaller right start, then the
    /// shorter ranges, then the smaller seed.
    fn cmp(&self, other: &Block) -> Ordering {
        self.alnum
            .cmp(&other.alnum)
            .then(self.pairs.cmp(&other.pairs))
            .then(other.l0.cmp(&self.l0))
            .then(other.r0.cmp(&self.r0))
            .then(other.l1.cmp(&self.l1))
            .then(other.r1.cmp(&self.r1))
            .then(other.seed.cmp(&self.seed))
    }
}

impl PartialOrd for Block {
    fn partial_cmp(&self, other: &Block) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

const NO_WEIGHT: i32 = i32::MIN;
/// Pair weight `2*sim - 2*T` of a seed: sim >= 9000 bp <=> weight >= 2000.
#[allow(clippy::cast_possible_wrap)]
const SEED_WEIGHT: i32 = 2 * (MOVE_EDIT_SEED_BP as i32) - 2 * (MOVE_EDITED_MIN_SIM_BP as i32);
/// Cost of an unpaired non-trivial line.
#[allow(clippy::cast_lossless)]
const GAP: i64 = MOVE_EDITED_MIN_SIM_BP as i64;
/// The alignment weight `2*sim - 2*T` of a pairable line pair (-4000 to +4000).
fn weight_of(sim: u32) -> i32 {
    let sim = i32::try_from(sim).unwrap_or(0);
    2 * sim - 2 * i32::try_from(MOVE_EDITED_MIN_SIM_BP).unwrap_or(0)
}

const CH_NONE: u8 = 0;
const CH_PAIR: u8 = 1;
const CH_UP: u8 = 2;
const CH_LEFT: u8 = 3;
const CH_CONT: u8 = 4;

/// The best block through one seed: ((first pair cell, last pair cell),
/// (score, pairs), seed cell).
type SeedBlock = ((usize, usize), (i64, u64), usize);

/// Both sweeps over one pair of segments.
struct Rect {
    sa: Seg,
    sb: Seg,
    fwd: Sweep,
    bwd: Sweep,
}

impl Rect {
    fn cols(&self) -> usize {
        self.sb.end - self.sb.start
    }

    fn cells(&self) -> usize {
        (self.sa.end - self.sa.start) * self.cols()
    }

    /// The best block through the `k`-th seed (in row-major order).
    fn through(&self, k: usize, seed_weight: i64) -> SeedBlock {
        let n = self.cells();
        let f = self.fwd.seeds[k];
        let b = self.bwd.seeds[self.bwd.seeds.len() - 1 - k];
        (
            (f.3, n - 1 - b.3),
            (f.1 + b.1 - seed_weight, f.2 + b.2 - 1),
            f.0,
        )
    }
}

/// One local-alignment sweep over a `rows x cols` rectangle.
struct Sweep {
    /// Per cell: the choice of `H` (low two bits) and whether the pair at the
    /// cell continues an earlier alignment (`CH_CONT`).
    choice: Vec<u8>,
    /// Seed cells in row-major order: (cell, score, pairs, origin cell).
    seeds: Vec<(usize, i64, u64, usize)>,
}

/// Local alignment where a pair `(r, c)` scores `w(r, c)` and an unpaired
/// non-trivial line scores `-GAP`. `H(r, c)` is the best alignment of a
/// rectangle ending at `(r, c)` with a non-negative score (ties: more pairs,
/// then pair, up, left); `P(r, c)` is the best one whose last pair is
/// `(r, c)`.
fn sweep(
    rows: usize,
    cols: usize,
    weight: impl Fn(usize, usize) -> i32,
    gap_row: impl Fn(usize) -> i64,
    gap_col: impl Fn(usize) -> i64,
) -> Sweep {
    let mut choice = vec![CH_NONE; rows * cols];
    let mut seeds = Vec::new();
    // Rolling rows of H: (score, pairs, origin); score None when undefined.
    let mut prev: Vec<Option<(i64, u64, usize)>> = vec![None; cols];
    let mut cur: Vec<Option<(i64, u64, usize)>> = vec![None; cols];
    for r in 0..rows {
        for c in 0..cols {
            let cell = r * cols + c;
            let mut best: Option<(i64, u64, usize)> = None;
            let mut ch = CH_NONE;
            let w = weight(r, c);
            if w != NO_WEIGHT {
                let mut p = (i64::from(w), 1, cell);
                if r > 0
                    && c > 0
                    && let Some(h) = prev[c - 1]
                {
                    p = (h.0 + i64::from(w), h.1 + 1, h.2);
                    choice[cell] |= CH_CONT;
                }
                if w >= SEED_WEIGHT {
                    seeds.push((cell, p.0, p.1, p.2));
                }
                best = Some(p);
                ch = CH_PAIR;
            }
            let better = |cand: (i64, u64, usize), best: Option<(i64, u64, usize)>| {
                best.is_none_or(|b| cand.0 > b.0 || (cand.0 == b.0 && cand.1 > b.1))
            };
            if r > 0
                && let Some(h) = prev[c]
            {
                let cand = (h.0 - gap_row(r), h.1, h.2);
                if better(cand, best) {
                    best = Some(cand);
                    ch = CH_UP;
                }
            }
            if c > 0
                && let Some(h) = cur[c - 1]
            {
                let cand = (h.0 - gap_col(c), h.1, h.2);
                if better(cand, best) {
                    best = Some(cand);
                    ch = CH_LEFT;
                }
            }
            if best.is_some_and(|b| b.0 < 0) {
                best = None;
                ch = CH_NONE;
            }
            choice[cell] |= ch;
            cur[c] = best;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    Sweep { choice, seeds }
}

/// Follows a sweep back from the pair at `(r, c)` to the first pair of its
/// alignment. Returns the pairs, last first.
fn trace(
    choice: &[u8],
    cols: usize,
    mut r: usize,
    mut c: usize,
    steps: &mut u64,
) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    'pairs: loop {
        pairs.push((r, c));
        *steps += 1;
        if choice[r * cols + c] & CH_CONT == 0 || r == 0 || c == 0 {
            break;
        }
        r -= 1;
        c -= 1;
        loop {
            *steps += 1;
            match choice[r * cols + c] & 3 {
                CH_PAIR => continue 'pairs,
                CH_UP if r > 0 => r -= 1,
                CH_LEFT if c > 0 => c -= 1,
                _ => break 'pairs,
            }
        }
    }
    pairs
}

struct Edited<'s, 'a, 'b> {
    ls: &'s mut Side<'a>,
    rs: &'s mut Side<'b>,
    check_delta: bool,
    /// Pairable right lines of each left line: (right line, similarity bp), by right line.
    pairable: Vec<Vec<(usize, u32)>>,
    heap: BinaryHeap<Block>,
    /// Sweeps made to repair candidates, by segment bounds. Segment bounds
    /// only change when lines are claimed, so an entry never goes stale.
    repairs: HashMap<(usize, usize, usize, usize), Rect>,
    cells: u64,
    exhausted: bool,
}

/// Runs the edited pass. Returns true when a work limit engaged.
fn edited_pass(
    ls: &mut Side<'_>,
    rs: &mut Side<'_>,
    key_texts: &[String],
    check_delta: bool,
    out: &mut Vec<DetectedMove>,
) -> bool {
    let lsegs = segments(ls);
    let rsegs = segments(rs);
    if lsegs.is_empty() || rsegs.is_empty() {
        return false;
    }
    // Segment pairs in different ops are compared smallest first (ties: left
    // start, then right start) while their line pairs fit in the budget, so
    // one large unrelated pair of ops does not switch the search off for the
    // small ones. `compared[a]` lists, by position, the right segments that
    // left segment `a` is compared with.
    if (lsegs.len() as u64) * (rsegs.len() as u64) > MOVE_EDIT_MAX_PAIRS {
        return true;
    }
    let mut order: Vec<(u64, usize, usize)> = Vec::new();
    for (a, sa) in lsegs.iter().enumerate() {
        for (b, sb) in rsegs.iter().enumerate() {
            if sa.op != sb.op {
                order.push((((sa.end - sa.start) * (sb.end - sb.start)) as u64, a, b));
            }
        }
    }
    order.sort_unstable();
    let mut compared: Vec<Vec<usize>> = vec![Vec::new(); lsegs.len()];
    let mut total = 0u64;
    let mut limited = false;
    for (area, a, b) in order {
        if total + area > MOVE_EDIT_MAX_PAIRS {
            limited = true;
            break;
        }
        total += area;
        compared[a].push(b);
    }
    for list in &mut compared {
        list.sort_unstable();
    }

    // Grapheme ids of the non-trivial lines in segments.
    let mut ids: HashMap<&str, u32> = HashMap::new();
    let mut graphemes = |side: &Side<'_>, segs: &[Seg]| -> Vec<Option<Graphemes>> {
        let mut out: Vec<Option<Graphemes>> = (0..side.len()).map(|_| None).collect();
        for s in segs {
            for i in s.start..s.end {
                if side.trivial(i) {
                    continue;
                }
                let text = key_texts[side.key[i] as usize].as_str();
                out[i] = Some(graphemes_of(text, &mut ids));
            }
        }
        out
    };
    let lgr = graphemes(ls, &lsegs);
    let rgr = graphemes(rs, &rsegs);
    let num_ids = ids.len();

    let mut pairable: Vec<Vec<(usize, u32)>> = vec![Vec::new(); ls.len()];
    let mut peq = vec![0u64; num_ids.max(1) * WORDS];
    for (sa, targets) in lsegs.iter().zip(&compared) {
        for i in sa.start..sa.end {
            let Some(a) = &lgr[i] else { continue };
            for (p, &g) in a.ids.iter().enumerate() {
                peq[g as usize * WORDS + p / 64] |= 1 << (p % 64);
            }
            for sb in targets.iter().map(|&b| &rsegs[b]) {
                for (j, b) in rgr.iter().enumerate().take(sb.end).skip(sb.start) {
                    let Some(b) = b else { continue };
                    let sim = if ls.key[i] == rs.key[j] {
                        Some(FULL_BP)
                    } else {
                        pair_similarity(a, b, &peq)
                    };
                    if let Some(sim) = sim {
                        pairable[i].push((j, sim));
                    }
                }
            }
            for &g in &a.ids {
                for w in 0..WORDS {
                    peq[g as usize * WORDS + w] = 0;
                }
            }
        }
    }

    let mut search = Edited {
        ls,
        rs,
        check_delta,
        pairable,
        heap: BinaryHeap::new(),
        repairs: HashMap::new(),
        cells: 0,
        exhausted: false,
    };
    // Every seed's best block over the initial segments, one per distinct
    // (first pair, last pair).
    for (sa, targets) in lsegs.iter().zip(&compared) {
        for sb in targets.iter().map(|&b| &rsegs[b]) {
            let Some(rect) = search.sweep_rect(*sa, *sb) else {
                continue;
            };
            let mut best: Vec<SeedBlock> = (0..rect.fwd.seeds.len())
                .map(|k| search.through_seed(&rect, k))
                .collect();
            best.sort_by(|x, y| {
                x.0.cmp(&y.0)
                    .then(y.1.0.cmp(&x.1.0))
                    .then(y.1.1.cmp(&x.1.1))
                    .then(x.2.cmp(&y.2))
            });
            best.dedup_by(|x, y| x.0 == y.0);
            let mut steps = 0;
            for (_, _, cell) in best {
                if let Some(block) = search.build(&rect, cell, &mut steps) {
                    search.heap.push(block);
                }
            }
            search.cells += steps;
        }
    }
    // Selection. A candidate that kept all its lines is still the best block
    // through its seed (the segments only shrank around it); one that lost
    // lines is rebuilt from its seed over the current segments.
    while let Some(block) = search.heap.pop() {
        let lost = (block.l0..block.l1).any(|i| search.ls.claimed[i])
            || (block.r0..block.r1).any(|j| search.rs.claimed[j]);
        if !lost {
            search.ls.claim(block.l0, block.l1 - block.l0);
            search.rs.claim(block.r0, block.r1 - block.r0);
            out.push(DetectedMove {
                left_start: block.l0,
                left_len: block.l1 - block.l0,
                right_start: block.r0,
                right_len: block.r1 - block.r0,
                kind: block.kind,
                similarity_bp: block.similarity_bp,
            });
        } else if let Some(block) = search.repair(block.seed) {
            search.heap.push(block);
        }
    }
    search.exhausted || limited
}

impl Edited<'_, '_, '_> {
    /// Sweeps one segment pair, if it holds a seed and the budget allows.
    fn sweep_rect(&mut self, sa: Seg, sb: Seg) -> Option<Rect> {
        if self.exhausted {
            return None;
        }
        let (rows, cols) = (sa.end - sa.start, sb.end - sb.start);
        let in_cols = |list: &[(usize, u32)]| -> (usize, usize) {
            let lo = list.partition_point(|p| p.0 < sb.start);
            let hi = list.partition_point(|p| p.0 < sb.end);
            (lo, hi)
        };
        let has_seed = (sa.start..sa.end).any(|i| {
            let list = &self.pairable[i];
            let (lo, hi) = in_cols(list);
            list[lo..hi].iter().any(|p| p.1 >= MOVE_EDIT_SEED_BP)
        });
        if !has_seed {
            return None;
        }
        let area = (rows * cols) as u64;
        if self.cells + 2 * area > MOVE_EDIT_MAX_CELLS {
            self.exhausted = true;
            return None;
        }
        self.cells += 2 * area;

        let mut weights = vec![NO_WEIGHT; rows * cols];
        for r in 0..rows {
            let list = &self.pairable[sa.start + r];
            let (lo, hi) = in_cols(list);
            for &(j, sim) in &list[lo..hi] {
                weights[r * cols + (j - sb.start)] = weight_of(sim);
            }
        }
        let gap_l: Vec<i64> = (sa.start..sa.end)
            .map(|i| if self.ls.trivial(i) { 0 } else { GAP })
            .collect();
        let gap_r: Vec<i64> = (sb.start..sb.end)
            .map(|j| if self.rs.trivial(j) { 0 } else { GAP })
            .collect();
        let n = rows * cols;
        let fwd = sweep(
            rows,
            cols,
            |r, c| weights[r * cols + c],
            |r| gap_l[r],
            |c| gap_r[c],
        );
        let bwd = sweep(
            rows,
            cols,
            |r, c| weights[n - 1 - (r * cols + c)],
            |r| gap_l[rows - 1 - r],
            |c| gap_r[cols - 1 - c],
        );
        Some(Rect { sa, sb, fwd, bwd })
    }

    /// The best block through the `k`-th seed of a rectangle.
    fn through_seed(&self, rect: &Rect, k: usize) -> SeedBlock {
        let cell = rect.fwd.seeds[k].0;
        let (r, c) = (cell / rect.cols(), cell % rect.cols());
        let sim = self
            .pair_sim(rect.sa.start + r, rect.sb.start + c)
            .unwrap_or(0);
        rect.through(k, i64::from(weight_of(sim)))
    }

    /// Traces the best alignment through the seed at `cell` and builds its block.
    fn build(&self, rect: &Rect, cell: usize, steps: &mut u64) -> Option<Block> {
        let (rows, cols) = (rect.sa.end - rect.sa.start, rect.cols());
        let (r, c) = (cell / cols, cell % cols);
        let mut pairs = trace(&rect.fwd.choice, cols, r, c, steps);
        pairs.reverse();
        let back = trace(&rect.bwd.choice, cols, rows - 1 - r, cols - 1 - c, steps);
        pairs.extend(
            back.iter()
                .skip(1)
                .map(|&(x, y)| (rows - 1 - x, cols - 1 - y)),
        );
        let core: Vec<(usize, usize, u32)> = pairs
            .iter()
            .map(|&(x, y)| {
                let (i, j) = (rect.sa.start + x, rect.sb.start + y);
                (i, j, self.pair_sim(i, j).unwrap_or(0))
            })
            .collect();
        let seed = (rect.sa.start + r, rect.sb.start + c);
        self.block((rect.sa, rect.sb), core, seed, steps)
    }

    /// Rebuilds the best block through a seed over the current segments.
    fn repair(&mut self, (i, j): (usize, usize)) -> Option<Block> {
        if self.ls.claimed[i] || self.rs.claimed[j] {
            return None;
        }
        let (sa, sb) = (segment_of(self.ls, i), segment_of(self.rs, j));
        let key = (sa.start, sa.end, sb.start, sb.end);
        if !self.repairs.contains_key(&key) {
            let rect = self.sweep_rect(sa, sb)?;
            self.repairs.insert(key, rect);
        }
        let rect = self.repairs.get(&key)?;
        let cell = (i - sa.start) * rect.cols() + (j - sb.start);
        rect.fwd.seeds.binary_search_by_key(&cell, |s| s.0).ok()?;
        let mut steps = 0;
        let block = self.build(rect, cell, &mut steps);
        self.cells += steps;
        block
    }

    /// Similarity of a pairable line pair.
    fn pair_sim(&self, i: usize, j: usize) -> Option<u32> {
        let list = &self.pairable[i];
        list.binary_search_by_key(&j, |p| p.0)
            .ok()
            .map(|k| list[k].1)
    }

    /// The best edge-extension step of a block, if one keeps the block
    /// similarity at or above the threshold: a pairable non-trivial pair just
    /// below (or above) the block that skips at most
    /// `MOVE_EDIT_EXTEND_SKIP` lines per side. Ranked by the resulting block
    /// similarity, then fewer skipped lines, then fewer skipped left lines;
    /// below wins a tie with above. Returns (left, right, sim, new sum, new n).
    fn extension(
        &self,
        (sa, sb): (Seg, Seg),
        (l0, l1, r0, r1): (usize, usize, usize, usize),
        (sum, n): (u64, u64),
        steps: &mut u64,
    ) -> Option<(usize, usize, u32, u64, u64)> {
        let (ls, rs) = (&*self.ls, &*self.rs);
        let nontrivial = |side: &Side<'_>, from: usize, to: usize| {
            (from..to).filter(|i| !side.trivial(*i)).count() as u64
        };
        let mut best: Option<(usize, usize, u32, u64, u64, usize, usize)> = None;
        let mut consider = |i: usize, j: usize, skip_l: usize, skip_r: usize, added: u64| {
            *steps += 1;
            if ls.trivial(i) || rs.trivial(j) {
                return;
            }
            let Some(sim) = self.pair_sim(i, j) else {
                return;
            };
            let (s2, n2) = (sum + u64::from(sim), n + added);
            if 2 * s2 < u64::from(MOVE_EDITED_MIN_SIM_BP) * n2 {
                return;
            }
            let better = best.is_none_or(|b| {
                let (lhs, rhs) = (
                    u128::from(s2) * u128::from(b.4),
                    u128::from(b.3) * u128::from(n2),
                );
                lhs > rhs || (lhs == rhs && (skip_l + skip_r, skip_l) < (b.5 + b.6, b.5))
            });
            if better {
                best = Some((i, j, sim, s2, n2, skip_l, skip_r));
            }
        };
        for skip_l in 0..=MOVE_EDIT_EXTEND_SKIP {
            for skip_r in 0..=MOVE_EDIT_EXTEND_SKIP {
                let (i, j) = (l1 + skip_l, r1 + skip_r);
                if i < sa.end && j < sb.end {
                    let added = nontrivial(ls, l1, i + 1) + nontrivial(rs, r1, j + 1);
                    consider(i, j, skip_l, skip_r, added);
                }
            }
        }
        for skip_l in 0..=MOVE_EDIT_EXTEND_SKIP {
            for skip_r in 0..=MOVE_EDIT_EXTEND_SKIP {
                if l0 > sa.start + skip_l && r0 > sb.start + skip_r {
                    let (i, j) = (l0 - 1 - skip_l, r0 - 1 - skip_r);
                    let added = nontrivial(ls, i, l0) + nontrivial(rs, j, r0);
                    consider(i, j, skip_l, skip_r, added);
                }
            }
        }
        best.map(|b| (b.0, b.1, b.2, b.3, b.4))
    }

    /// Builds the block for an alignment (absolute line pairs with their
    /// similarity, in order): ranges from the first to the last pair, grown
    /// by edge extension while the block similarity stays at or above the
    /// threshold, extended over matching trivial edge lines and trimmed of
    /// blank edge lines.
    fn block(
        &self,
        (sa, sb): (Seg, Seg),
        mut pairs: Vec<(usize, usize, u32)>,
        seed: (usize, usize),
        steps: &mut u64,
    ) -> Option<Block> {
        let (first, last) = (*pairs.first()?, *pairs.last()?);
        let (mut l0, mut r0) = (first.0, first.1);
        let (mut l1, mut r1) = (last.0 + 1, last.1 + 1);
        let (ls, rs) = (&*self.ls, &*self.rs);
        let nontrivial = |side: &Side<'_>, from: usize, to: usize| {
            (from..to).filter(|i| !side.trivial(*i)).count() as u64
        };
        let mut sum: u64 = pairs.iter().map(|p| u64::from(p.2)).sum();
        let mut n = nontrivial(ls, l0, l1) + nontrivial(rs, r0, r1);
        while let Some((i, j, sim, s2, n2)) =
            self.extension((sa, sb), (l0, l1, r0, r1), (sum, n), steps)
        {
            (sum, n) = (s2, n2);
            if i >= l1 {
                (l1, r1) = (i + 1, j + 1);
                pairs.push((i, j, sim));
            } else {
                (l0, r0) = (i, j);
                pairs.insert(0, (i, j, sim));
            }
        }
        let edge = |i: usize, j: usize| ls.trivial(i) && rs.trivial(j) && ls.key[i] == rs.key[j];
        let (core_l0, core_l1) = (l0, l1);
        while l0 > sa.start && r0 > sb.start && edge(l0 - 1, r0 - 1) {
            l0 -= 1;
            r0 -= 1;
        }
        while l0 < core_l0 && ls.blank[l0] {
            l0 += 1;
            r0 += 1;
        }
        while l1 < sa.end && r1 < sb.end && edge(l1, r1) {
            l1 += 1;
            r1 += 1;
        }
        while l1 > core_l1 && ls.blank[l1 - 1] {
            l1 -= 1;
            r1 -= 1;
        }
        if !ls.qualifies(l0, l1 - l0) || !rs.qualifies(r0, r1 - r0) {
            return None;
        }
        // Trivial edge lines do not change `n` or `sum`.
        let alnum: u64 = pairs.iter().map(|p| u64::from(ls.alnum[p.0])).sum();
        let similarity = 2 * sum / n.max(1);
        if similarity < u64::from(MOVE_EDITED_MIN_SIM_BP) {
            return None;
        }
        let pure = l1 - l0 == r1 - r0 && is_pure(ls, rs, l0, r0, l1 - l0, self.check_delta);
        Some(Block {
            alnum,
            pairs: pairs.len() as u64,
            l0,
            l1,
            r0,
            r1,
            seed,
            kind: if pure {
                MoveKind::Pure
            } else {
                MoveKind::Edited
            },
            similarity_bp: if pure {
                FULL_BP
            } else {
                u32::try_from(similarity.min(u64::from(FULL_BP))).unwrap_or(FULL_BP)
            },
        })
    }
}

#[cfg(test)]
mod tests;
