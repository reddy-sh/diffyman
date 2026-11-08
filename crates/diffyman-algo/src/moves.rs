//! Moved-block detection (`docs/spec/text-granularity-and-moves.md` §4),
//! algorithm ID `text.moves@1`.
//!
//! **Phase 0 stub.** [`detect_moves`] always returns no moves. Phase 1 builds
//! the detector against the labeled corpus in `tests/corpus/moved/`. The
//! constants below are the spec values the detector will use.

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
}

/// Detects moved blocks among deleted and inserted lines.
///
/// Phase 0 stub: returns an empty list for every input.
pub fn detect_moves(_left: &[&str], _right: &[&str], _ops: &[LineOp]) -> Vec<DetectedMove> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_returns_no_moves() {
        assert!(detect_moves(&["a"], &["a"], &[]).is_empty());
    }
}
