//! Inline refinement of `replace` ops (`docs/spec/text-granularity-and-moves.md`
//! §3), algorithm ID `text.refine@1`. The golden text cases pin its output
//! for `word`, `char` and `smart`.
//!
//! Where the spec text is ambiguous, this code is the reference:
//!
//! - Tokens are UAX #29 word segments (`word`) or extended grapheme clusters
//!   (`char`). A run of adjacent whitespace segments is one whitespace token.
//!   Whitespace means U+0009, U+000B, U+000C and U+0020, as for line keys.
//! - Each line terminator is one token equal to any other terminator. It never
//!   produces a span, because spans cover line content only.
//! - Under whitespace mode `all`, whitespace tokens are left out of the token
//!   diff; under `trailing` and `change`, trailing whitespace tokens are left
//!   out; under `change`, the other whitespace tokens compare as one space.
//! - The `smart` noise cutoff (§3.3 step 4) counts, per side, the
//!   non-whitespace graphemes outside every span left by steps 1 to 3, and
//!   uses their sum `eqL + eqR` in place of `2*eq`.
//! - The token limit applies to the token stream of the level actually used.

use std::collections::HashMap;

use icu_properties::CodePointMapData;
use icu_properties::props::Script;
use unicode_segmentation::UnicodeSegmentation;

use crate::key::{WhitespaceMode, is_whitespace, simple_fold};
use crate::line_diff::align_tokens_myers;
use crate::lines::Terminator;
use crate::ops::full_ops;

/// Algorithm ID of this refinement.
pub const REFINE_ID: &str = "text.refine@1";
/// Most lines per side that get inline spans.
pub const REFINE_MAX_LINES: usize = 100;
/// Most UTF-8 bytes per side (content plus terminators).
pub const REFINE_MAX_BYTES: usize = 16_384;
/// Most tokens per side.
pub const REFINE_MAX_TOKENS: usize = 4_096;
/// Minimum share (percent) of spaceless-script graphemes for char-level `smart`.
pub const SMART_SPACELESS_MIN_PCT: u64 = 30;
/// Minimum graphemes per token for the sub-word pass.
pub const SMART_SUBWORD_MIN_GRAPHEMES: usize = 4;
/// Minimum similarity (percent) for sub-word spans.
pub const SMART_SUBWORD_MIN_SIM_PCT: u64 = 50;
/// Minimum equal share (percent) below which `smart` emits no spans.
pub const SMART_MIN_EQUAL_PCT: u64 = 25;
/// Scripts written without spaces between words.
pub const SMART_SPACELESS_SCRIPTS: [Script; 7] = [
    Script::Han,
    Script::Hiragana,
    Script::Katakana,
    Script::Thai,
    Script::Lao,
    Script::Khmer,
    Script::Myanmar,
];

/// Inline granularity (the `line` mode never reaches this module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    /// UAX #29 word segments.
    Word,
    /// Extended grapheme clusters.
    Char,
    /// Chosen per op (§3.3).
    Smart,
}

/// Unit of inline offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionEncoding {
    /// UTF-8 bytes.
    #[default]
    Utf8,
    /// UTF-16 code units.
    Utf16,
    /// Unicode scalar values.
    Codepoint,
}

/// Options for refinement.
#[derive(Debug, Clone)]
pub struct RefineOptions {
    /// Granularity.
    pub granularity: Granularity,
    /// Whitespace mode of the document.
    pub whitespace: WhitespaceMode,
    /// Case folding.
    pub ignore_case: bool,
    /// Offset unit.
    pub position_encoding: PositionEncoding,
}

/// One line of an op side.
#[derive(Debug, Clone, Copy)]
pub struct SideLine<'a> {
    /// Line content without its terminator.
    pub content: &'a str,
    /// The line's terminator.
    pub terminator: Terminator,
}

/// Which side a span is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpanSide {
    /// Left (a deletion).
    Left,
    /// Right (an insertion).
    Right,
}

/// An inline span. `line` is the 0-based index of the line within the op side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InlineSpan {
    /// Side.
    pub side: SpanSide,
    /// Line index within the op side.
    pub line: usize,
    /// Start offset in the line content.
    pub start: u64,
    /// End offset (exclusive).
    pub end: u64,
}

/// Result of refining one op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refined {
    /// Spans, left first, each side sorted by `(line, start)`. May be empty.
    Spans(Vec<InlineSpan>),
    /// The op is over a size limit; it gets no spans.
    SkippedLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Word,
    Char,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokKind {
    Text,
    Space,
    Eol,
}

#[derive(Debug, Clone, Copy)]
struct Tok {
    line: usize,
    start: usize,
    end: usize,
    kind: TokKind,
}

/// A byte range inside one line's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ByteSpan {
    line: usize,
    start: usize,
    end: usize,
}

/// Refines one `replace` op.
pub fn refine(left: &[SideLine<'_>], right: &[SideLine<'_>], options: &RefineOptions) -> Refined {
    if left.is_empty() || right.is_empty() {
        return Refined::Spans(Vec::new());
    }
    let side_bytes = |side: &[SideLine<'_>]| -> usize {
        side.iter()
            .map(|l| l.content.len() + l.terminator.len())
            .sum()
    };
    if left.len() > REFINE_MAX_LINES
        || right.len() > REFINE_MAX_LINES
        || side_bytes(left) > REFINE_MAX_BYTES
        || side_bytes(right) > REFINE_MAX_BYTES
    {
        return Refined::SkippedLimit;
    }
    let level = match options.granularity {
        Granularity::Word => Level::Word,
        Granularity::Char => Level::Char,
        Granularity::Smart => {
            if is_spaceless(left, right) {
                Level::Char
            } else {
                Level::Word
            }
        }
    };
    let lt = tokenize(left, level);
    let rt = tokenize(right, level);
    if lt.len() > REFINE_MAX_TOKENS || rt.len() > REFINE_MAX_TOKENS {
        return Refined::SkippedLimit;
    }

    let smart = options.granularity == Granularity::Smart;
    let mut keys = KeyTable::new(options.ignore_case);
    let (l_idx, l_ids) = comparison_stream(left, &lt, options, &mut keys);
    let (r_idx, r_ids) = comparison_stream(right, &rt, options, &mut keys);
    let alignment = align_tokens_myers(&l_ids, &r_ids);

    let mut l_changed = vec![false; lt.len()];
    let mut r_changed = vec![false; rt.len()];
    for (k, &removed) in alignment.removed.iter().enumerate() {
        l_changed[l_idx[k]] = removed;
    }
    for (k, &added) in alignment.added.iter().enumerate() {
        r_changed[r_idx[k]] = added;
    }

    // Sub-word pass (smart, word level only): token index -> char-level ranges.
    let mut l_partial: HashMap<usize, Vec<ByteSpan>> = HashMap::new();
    let mut r_partial: HashMap<usize, Vec<ByteSpan>> = HashMap::new();
    if smart && level == Level::Word {
        for op in full_ops(&alignment) {
            if op.left_len != 1 || op.right_len != 1 {
                continue;
            }
            let (li, ri) = (l_idx[op.left_start], r_idx[op.right_start]);
            let (a, b) = (lt[li], rt[ri]);
            let a_text = &left[a.line].content[a.start..a.end];
            let b_text = &right[b.line].content[b.start..b.end];
            if a.kind != TokKind::Text
                || b.kind != TokKind::Text
                || !is_word_like(a_text)
                || !is_word_like(b_text)
            {
                continue;
            }
            if let Some((ls, rs)) = subword(a, a_text, b, b_text, options.ignore_case) {
                l_partial.insert(li, ls);
                r_partial.insert(ri, rs);
            }
        }
    }

    let ws_merge = options.granularity != Granularity::Char;
    let l_spans = collect_spans(&lt, &l_changed, &l_partial, ws_merge);
    let r_spans = collect_spans(&rt, &r_changed, &r_partial, ws_merge);

    if smart && is_noise(left, &l_spans, right, &r_spans) {
        return Refined::Spans(Vec::new());
    }

    let mut out = Vec::with_capacity(l_spans.len() + r_spans.len());
    for (side, spans, lines) in [
        (SpanSide::Left, &l_spans, left),
        (SpanSide::Right, &r_spans, right),
    ] {
        for s in spans {
            let content = lines[s.line].content;
            out.push(InlineSpan {
                side,
                line: s.line,
                start: units(content, s.start, options.position_encoding),
                end: units(content, s.end, options.position_encoding),
            });
        }
    }
    Refined::Spans(out)
}

/// §3.3 step 1: share of spaceless-script graphemes among non-whitespace graphemes.
fn is_spaceless(left: &[SideLine<'_>], right: &[SideLine<'_>]) -> bool {
    let scripts = CodePointMapData::<Script>::new();
    let (mut total, mut spaceless) = (0_u64, 0_u64);
    for line in left.iter().chain(right) {
        for g in line.content.graphemes(true) {
            if g.chars().all(is_whitespace) {
                continue;
            }
            total += 1;
            if let Some(c) = g.chars().next()
                && SMART_SPACELESS_SCRIPTS.contains(&scripts.get(c))
            {
                spaceless += 1;
            }
        }
    }
    spaceless > 0 && 100 * spaceless >= SMART_SPACELESS_MIN_PCT * total
}

/// §3.3 step 4: true when too few non-whitespace graphemes are left equal by
/// the earlier steps, that is, lie outside every span.
fn is_noise(
    left: &[SideLine<'_>],
    l_spans: &[ByteSpan],
    right: &[SideLine<'_>],
    r_spans: &[ByteSpan],
) -> bool {
    let (l_total, l_eq) = equal_graphemes(left, l_spans);
    let (r_total, r_eq) = equal_graphemes(right, r_spans);
    (l_eq + r_eq) * 100 < SMART_MIN_EQUAL_PCT * (l_total + r_total)
}

/// Non-whitespace graphemes of one side: (all, outside every span).
fn equal_graphemes(lines: &[SideLine<'_>], spans: &[ByteSpan]) -> (u64, u64) {
    let (mut total, mut equal) = (0_u64, 0_u64);
    for (li, line) in lines.iter().enumerate() {
        for (off, g) in line.content.grapheme_indices(true) {
            if g.chars().all(is_whitespace) {
                continue;
            }
            total += 1;
            let changed = spans
                .iter()
                .any(|s| s.line == li && s.start <= off && off < s.end);
            if !changed {
                equal += 1;
            }
        }
    }
    (total, equal)
}

/// §3.3 step 3 for one pair of word-like tokens. Returns char-level ranges for
/// both sides when the tokens are similar enough.
fn subword(
    a: Tok,
    a_text: &str,
    b: Tok,
    b_text: &str,
    ignore_case: bool,
) -> Option<(Vec<ByteSpan>, Vec<ByteSpan>)> {
    let ag: Vec<(usize, &str)> = a_text.grapheme_indices(true).collect();
    let bg: Vec<(usize, &str)> = b_text.grapheme_indices(true).collect();
    if ag.len() < SMART_SUBWORD_MIN_GRAPHEMES || bg.len() < SMART_SUBWORD_MIN_GRAPHEMES {
        return None;
    }
    let mut keys = KeyTable::new(ignore_case);
    let a_ids: Vec<u32> = ag.iter().map(|(_, g)| keys.text(g)).collect();
    let b_ids: Vec<u32> = bg.iter().map(|(_, g)| keys.text(g)).collect();
    let alignment = align_tokens_myers(&a_ids, &b_ids);
    let eq = alignment.removed.iter().filter(|x| !**x).count() as u64;
    if 2 * eq * 100 < SMART_SUBWORD_MIN_SIM_PCT * (ag.len() + bg.len()) as u64 {
        return None;
    }
    let ranges = |tok: Tok, g: &[(usize, &str)], changed: &[bool]| -> Vec<ByteSpan> {
        g.iter()
            .zip(changed)
            .filter(|(_, c)| **c)
            .map(|((off, s), _)| ByteSpan {
                line: tok.line,
                start: tok.start + off,
                end: tok.start + off + s.len(),
            })
            .collect()
    };
    Some((
        ranges(a, &ag, &alignment.removed),
        ranges(b, &bg, &alignment.added),
    ))
}

fn is_word_like(token: &str) -> bool {
    token.chars().any(char::is_alphanumeric)
}

fn tokenize(lines: &[SideLine<'_>], level: Level) -> Vec<Tok> {
    let mut toks: Vec<Tok> = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let segments: Vec<(usize, &str)> = match level {
            Level::Word => line.content.split_word_bound_indices().collect(),
            Level::Char => line.content.grapheme_indices(true).collect(),
        };
        for (off, seg) in segments {
            let space = seg.chars().all(is_whitespace);
            if space
                && let Some(last) = toks.last_mut()
                && last.kind == TokKind::Space
                && last.line == li
                && last.end == off
            {
                last.end = off + seg.len();
                continue;
            }
            toks.push(Tok {
                line: li,
                start: off,
                end: off + seg.len(),
                kind: if space { TokKind::Space } else { TokKind::Text },
            });
        }
        if !line.terminator.is_empty() {
            let len = line.content.len();
            toks.push(Tok {
                line: li,
                start: len,
                end: len,
                kind: TokKind::Eol,
            });
        }
    }
    toks
}

/// Interns token keys. ID 0 is the line terminator.
struct KeyTable {
    ids: HashMap<String, u32>,
    ignore_case: bool,
}

impl KeyTable {
    fn new(ignore_case: bool) -> KeyTable {
        KeyTable {
            ids: HashMap::new(),
            ignore_case,
        }
    }

    fn text(&mut self, text: &str) -> u32 {
        let key: String = if self.ignore_case {
            text.chars().map(simple_fold).collect()
        } else {
            text.to_owned()
        };
        let next = u32::try_from(self.ids.len() + 1).unwrap_or(u32::MAX);
        *self.ids.entry(key).or_insert(next)
    }
}

/// The tokens that take part in the diff (their indices) and their key IDs.
fn comparison_stream(
    lines: &[SideLine<'_>],
    toks: &[Tok],
    options: &RefineOptions,
    keys: &mut KeyTable,
) -> (Vec<usize>, Vec<u32>) {
    let mut idx = Vec::with_capacity(toks.len());
    let mut ids = Vec::with_capacity(toks.len());
    for (i, tok) in toks.iter().enumerate() {
        let id = match tok.kind {
            TokKind::Eol => Some(0),
            TokKind::Text => Some(keys.text(&lines[tok.line].content[tok.start..tok.end])),
            TokKind::Space => {
                let trailing = toks
                    .get(i + 1)
                    .is_none_or(|next| next.line != tok.line || next.kind == TokKind::Eol);
                let excluded = match options.whitespace {
                    WhitespaceMode::All => true,
                    WhitespaceMode::Trailing | WhitespaceMode::Change => trailing,
                    WhitespaceMode::None => false,
                };
                if excluded {
                    None
                } else if options.whitespace == WhitespaceMode::Change {
                    Some(keys.text(" "))
                } else {
                    Some(keys.text(&lines[tok.line].content[tok.start..tok.end]))
                }
            }
        };
        if let Some(id) = id {
            idx.push(i);
            ids.push(id);
        }
    }
    (idx, ids)
}

/// Turns changed tokens into merged byte spans for one side.
fn collect_spans(
    toks: &[Tok],
    changed: &[bool],
    partial: &HashMap<usize, Vec<ByteSpan>>,
    ws_merge: bool,
) -> Vec<ByteSpan> {
    let mut raw: Vec<ByteSpan> = Vec::new();
    for (i, tok) in toks.iter().enumerate() {
        if tok.kind == TokKind::Eol {
            continue;
        }
        if let Some(parts) = partial.get(&i) {
            raw.extend_from_slice(parts);
        } else if changed[i] {
            raw.push(ByteSpan {
                line: tok.line,
                start: tok.start,
                end: tok.end,
            });
        }
    }
    let space_between = |line: usize, start: usize, end: usize| {
        toks.iter()
            .any(|t| t.kind == TokKind::Space && t.line == line && t.start == start && t.end == end)
    };
    let mut merged: Vec<ByteSpan> = Vec::with_capacity(raw.len());
    for span in raw {
        if let Some(last) = merged.last_mut()
            && last.line == span.line
            && (last.end == span.start
                || (ws_merge && space_between(span.line, last.end, span.start)))
        {
            last.end = span.end;
            continue;
        }
        merged.push(span);
    }
    merged
}

/// Converts a byte offset within `content` to the requested unit.
fn units(content: &str, byte_offset: usize, encoding: PositionEncoding) -> u64 {
    let prefix = &content[..byte_offset];
    let n = match encoding {
        PositionEncoding::Utf8 => byte_offset,
        PositionEncoding::Utf16 => prefix.encode_utf16().count(),
        PositionEncoding::Codepoint => prefix.chars().count(),
    };
    n as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(content: &str) -> SideLine<'_> {
        SideLine {
            content,
            terminator: Terminator::Lf,
        }
    }

    fn opts(granularity: Granularity) -> RefineOptions {
        RefineOptions {
            granularity,
            whitespace: WhitespaceMode::None,
            ignore_case: false,
            position_encoding: PositionEncoding::Utf8,
        }
    }

    /// Spans as (side, line, text) for readable assertions.
    fn texts<'a>(
        left: &[&'a str],
        right: &[&'a str],
        o: &RefineOptions,
    ) -> Vec<(SpanSide, usize, &'a str)> {
        let l: Vec<SideLine<'a>> = left.iter().map(|s| line(s)).collect();
        let r: Vec<SideLine<'a>> = right.iter().map(|s| line(s)).collect();
        let Refined::Spans(spans) = refine(&l, &r, o) else {
            panic!("skipped");
        };
        assert_eq!(o.position_encoding, PositionEncoding::Utf8);
        spans
            .iter()
            .map(|s| {
                let src = if s.side == SpanSide::Left {
                    left
                } else {
                    right
                };
                let text = &src[s.line]
                    [usize::try_from(s.start).unwrap()..usize::try_from(s.end).unwrap()];
                (s.side, s.line, text)
            })
            .collect()
    }

    use SpanSide::{Left as L, Right as R};

    #[test]
    fn smart_subword() {
        assert_eq!(
            texts(&["colour"], &["color"], &opts(Granularity::Smart)),
            [(L, 0, "u")]
        );
    }

    #[test]
    fn smart_identifier() {
        assert_eq!(
            texts(
                &["getUserName()"],
                &["getUsername()"],
                &opts(Granularity::Smart)
            ),
            [(L, 0, "N"), (R, 0, "n")]
        );
    }

    #[test]
    fn smart_short_word() {
        assert_eq!(
            texts(&["foo bar"], &["foo baz"], &opts(Granularity::Smart)),
            [(L, 0, "bar"), (R, 0, "baz")]
        );
    }

    #[test]
    fn smart_two_words_merge_across_space() {
        assert_eq!(
            texts(
                &["I ate a red apple"],
                &["I ate a green pear"],
                &opts(Granularity::Smart)
            ),
            [(L, 0, "red apple"), (R, 0, "green pear")]
        );
        // Without shared words the whole op is noise.
        assert!(texts(&["red apple"], &["green pear"], &opts(Granularity::Smart)).is_empty());
    }

    #[test]
    fn smart_cjk_and_thai() {
        assert_eq!(
            texts(&["我喜欢猫"], &["我喜欢狗"], &opts(Granularity::Smart)),
            [(L, 0, "猫"), (R, 0, "狗")]
        );
        // "I like cats" -> "I like dogs" in Thai: char-level spans.
        let spans = texts(&["ฉันชอบแมว"], &["ฉันชอบหมา"], &opts(Granularity::Smart));
        assert!(!spans.is_empty());
        assert!(spans.iter().all(|(_, _, t)| t.chars().count() <= 3));
    }

    #[test]
    fn smart_noise_cutoff() {
        assert!(
            texts(
                &["xyzzy 12345"],
                &["qwvjk 67890"],
                &opts(Granularity::Smart)
            )
            .is_empty()
        );
        // Unrelated ordinary text shares letters, but no words survive the
        // word pass.
        assert!(
            texts(
                &["the quick brown fox"],
                &["lorem ipsum dolor"],
                &opts(Granularity::Smart)
            )
            .is_empty()
        );
        // One shared word in four is under the cutoff too.
        assert!(
            texts(
                &["the quick brown fox"],
                &["the lazy sleepy dog"],
                &opts(Granularity::Smart)
            )
            .is_empty()
        );
        // Explicit word mode still shows spans.
        assert!(!texts(&["xyzzy 12345"], &["qwvjk 67890"], &opts(Granularity::Word)).is_empty());
    }

    #[test]
    fn smart_emoji_cluster() {
        assert_eq!(
            texts(
                &["👩\u{200d}💻 ok"],
                &["👨\u{200d}💻 ok"],
                &opts(Granularity::Smart)
            ),
            [(L, 0, "👩\u{200d}💻"), (R, 0, "👨\u{200d}💻")]
        );
    }

    #[test]
    fn too_big_is_skipped() {
        let many: Vec<SideLine<'_>> = (0..=REFINE_MAX_LINES).map(|_| line("a")).collect();
        assert_eq!(
            refine(&many, &[line("b")], &opts(Granularity::Smart)),
            Refined::SkippedLimit
        );
    }

    #[test]
    fn char_mode_does_not_merge_across_space() {
        assert_eq!(
            texts(&["a b"], &["c d"], &opts(Granularity::Char)),
            [(L, 0, "a"), (L, 0, "b"), (R, 0, "c"), (R, 0, "d")]
        );
    }

    #[test]
    fn multi_line_ops() {
        assert_eq!(
            texts(
                &["one two", "three"],
                &["one too", "three", "four"],
                &opts(Granularity::Word)
            ),
            [(L, 0, "two"), (R, 0, "too"), (R, 2, "four")]
        );
    }

    #[test]
    fn whitespace_modes_hide_space_changes() {
        let mut o = opts(Granularity::Word);
        o.whitespace = WhitespaceMode::Change;
        assert_eq!(
            texts(&["a  b x"], &["a b y"], &o),
            [(L, 0, "x"), (R, 0, "y")]
        );
        // Under `all`, spaces take no part in the diff, so changed tokens on
        // either side of one merge into a single span.
        o.whitespace = WhitespaceMode::All;
        assert_eq!(
            texts(&["ab x"], &["a b y"], &o),
            [(L, 0, "ab x"), (R, 0, "a b y")]
        );
    }

    #[test]
    fn ignore_case_in_tokens() {
        let mut o = opts(Granularity::Word);
        o.ignore_case = true;
        assert_eq!(
            texts(&["Hello World"], &["hello there"], &o),
            [(L, 0, "World"), (R, 0, "there")]
        );
    }

    #[test]
    fn position_encodings() {
        let l = [line("é猫x")];
        let r = [line("é猫y")];
        let mut o = opts(Granularity::Char);
        let at = |o: &RefineOptions| match refine(&l, &r, o) {
            Refined::Spans(s) => (s[0].start, s[0].end),
            Refined::SkippedLimit => panic!(),
        };
        assert_eq!(at(&o), (5, 6));
        o.position_encoding = PositionEncoding::Utf16;
        assert_eq!(at(&o), (2, 3));
        o.position_encoding = PositionEncoding::Codepoint;
        assert_eq!(at(&o), (2, 3));
        let astral = [line("😀x")];
        let astral_r = [line("😀y")];
        o.position_encoding = PositionEncoding::Utf16;
        let Refined::Spans(s) = refine(&astral, &astral_r, &o) else {
            panic!()
        };
        assert_eq!((s[0].start, s[0].end), (2, 3));
    }
}
