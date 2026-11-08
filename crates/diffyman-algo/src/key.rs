//! Line comparison keys and interning (Diff Document spec §7.2).
//!
//! Two lines are equal when their keys are equal. The key is built from the
//! line content in this order: regex masks, whitespace mode, case folding, and
//! (for whitespace mode `none` only) the terminator kind.
//!
//! A regex mask replaces each match with a placeholder for its pattern
//! (spec §7.2 step 1). In the key a placeholder is the byte `0xFE` followed by
//! the pattern index; `0xFE` never occurs in UTF-8, so a placeholder cannot
//! collide with text, and two placeholders are equal only for the same pattern.

use std::collections::HashMap;

use icu_casemap::{CaseMapper, CaseMapperBorrowed};
use regex::{Regex, RegexBuilder};
use regex_automata::nfa::thompson;

use crate::lines::{Line, LineIndex, Terminator};

/// Most patterns `ignore.regex` may hold.
pub const MAX_REGEX_PATTERNS: usize = 16;
/// Longest pattern, in Unicode scalar values.
pub const MAX_REGEX_PATTERN_CHARS: usize = 1024;
/// Largest compiled pattern, in NFA units: the states plus the transitions of
/// its forward Thompson NFA ([`nfa_units`]). The count does not depend on the
/// platform, so the same pattern is accepted or rejected everywhere.
pub const MAX_REGEX_NFA_UNITS: usize = 50_000;
/// Byte limit handed to the regex compilers. A pattern within
/// [`MAX_REGEX_NFA_UNITS`] needs at most a few MB on any platform, so this limit
/// never decides on its own; it only bounds memory for larger patterns.
pub const REGEX_SIZE_LIMIT: usize = 16 << 20;

/// Key byte that starts a placeholder; the next byte is the pattern index.
/// It never occurs in UTF-8.
pub const MASK_BYTE: u8 = 0xFE;

/// Whitespace handling for line comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhitespaceMode {
    /// Content plus terminator kind.
    #[default]
    None,
    /// Trailing whitespace removed; terminator ignored.
    Trailing,
    /// Trailing whitespace removed, other runs collapsed to one space; terminator ignored.
    Change,
    /// All whitespace removed; terminator ignored.
    All,
}

/// Whitespace for comparison: only U+0009, U+000B, U+000C and U+0020.
pub fn is_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\u{b}' | '\u{c}' | ' ')
}

/// True when `content` is empty or whitespace only.
pub fn is_blank(content: &str) -> bool {
    content.chars().all(is_whitespace)
}

/// Inputs to key building.
#[derive(Debug, Clone, Default)]
pub struct KeyOptions {
    /// Whitespace mode.
    pub whitespace: WhitespaceMode,
    /// Apply Unicode simple case folding.
    pub ignore_case: bool,
    /// Regex patterns whose matches are masked, in priority order.
    pub regex: Vec<String>,
}

/// Why `ignore.regex` could not be compiled. Both flags can be set when
/// several patterns fail for different reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MaskErrors {
    /// A pattern is not a valid regular expression (`invalid-regex`).
    pub invalid: bool,
    /// Too many patterns, a pattern too long, or a compiled pattern too large
    /// (`limit-exceeded`).
    pub limit: bool,
}

/// Builds comparison keys for lines.
#[derive(Debug, Clone)]
pub struct KeyBuilder {
    whitespace: WhitespaceMode,
    fold: Option<CaseMapperBorrowed<'static>>,
    masks: Vec<Regex>,
    /// Finds two spaces in a row (whitespace mode `change`).
    double_space: memchr::memmem::Finder<'static>,
}

/// Applies Unicode simple case folding (CaseFolding.txt statuses C and S).
pub fn simple_fold(c: char) -> char {
    if c.is_ascii() {
        // CaseFolding.txt maps only `A`–`Z` among ASCII characters.
        c.to_ascii_lowercase()
    } else {
        CaseMapper::new().simple_fold(c)
    }
}

/// States plus transitions of an NFA: a platform-independent size measure.
pub fn nfa_units(nfa: &thompson::NFA) -> usize {
    nfa.states()
        .iter()
        .map(|state| {
            1 + match state {
                thompson::State::Sparse(sparse) => sparse.transitions.len(),
                thompson::State::Dense(_) => 256,
                thompson::State::Union { alternates } => alternates.len(),
                _ => 0,
            }
        })
        .sum()
}

/// Compiles one mask pattern within the limits.
fn compile_mask(pattern: &str) -> Result<Regex, MaskErrors> {
    let invalid = MaskErrors {
        invalid: true,
        limit: false,
    };
    let limit = MaskErrors {
        invalid: false,
        limit: true,
    };
    if pattern.chars().count() > MAX_REGEX_PATTERN_CHARS {
        return Err(limit);
    }
    // Measure first: the unit count is the same on every platform, unlike the
    // byte sizes the compilers use for their own limits.
    let hir = regex_automata::util::syntax::parse(pattern).map_err(|_| invalid)?;
    let nfa = thompson::Compiler::new()
        .configure(thompson::Config::new().nfa_size_limit(Some(REGEX_SIZE_LIMIT)))
        .build_from_hir(&hir)
        .map_err(|_| limit)?;
    if nfa_units(&nfa) > MAX_REGEX_NFA_UNITS {
        return Err(limit);
    }
    RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| match e {
            regex::Error::CompiledTooBig(_) => limit,
            _ => invalid,
        })
}

/// Compiles `ignore.regex` (spec §9): at most [`MAX_REGEX_PATTERNS`]
/// patterns, each valid and within the size limits.
pub fn compile_masks(patterns: &[String]) -> Result<Vec<Regex>, MaskErrors> {
    if patterns.len() > MAX_REGEX_PATTERNS {
        return Err(MaskErrors {
            invalid: false,
            limit: true,
        });
    }
    let mut errors = MaskErrors::default();
    let mut masks = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        match compile_mask(pattern) {
            Ok(regex) => masks.push(regex),
            Err(e) => {
                errors.invalid |= e.invalid;
                errors.limit |= e.limit;
            }
        }
    }
    if errors == MaskErrors::default() {
        Ok(masks)
    } else {
        Err(errors)
    }
}

impl KeyBuilder {
    /// Compiles the options. Fails only on `ignore.regex` problems.
    pub fn new(options: &KeyOptions) -> Result<KeyBuilder, MaskErrors> {
        Ok(KeyBuilder {
            whitespace: options.whitespace,
            fold: options.ignore_case.then(CaseMapper::new),
            masks: compile_masks(&options.regex)?,
            double_space: memchr::memmem::Finder::new(b"  "),
        })
    }

    /// The masked parts of `content` as `(start, end, pattern)` byte ranges,
    /// sorted and disjoint (spec §7.2 step 1). See [`masked_ranges`].
    pub fn masked_ranges(&self, content: &str) -> Vec<(usize, usize, u8)> {
        masked_ranges(&self.masks, content)
    }

    /// The compiled `ignore.regex` patterns, in priority order.
    pub fn masks(&self) -> &[Regex] {
        &self.masks
    }

    /// The whitespace mode.
    pub fn whitespace(&self) -> WhitespaceMode {
        self.whitespace
    }

    /// The move key of one line (moves spec §4.4), written into `out`
    /// (cleared first): the comparison key of spec §7.2 steps 1 to 4 without
    /// the terminator, with the leading whitespace (the whitespace before the
    /// first other character or placeholder) removed first. Placeholders are
    /// written as in [`KeyBuilder::key_into`].
    pub fn move_key_into(&self, content: &str, out: &mut Vec<u8>) {
        fn lead(piece: &str, first: bool) -> &str {
            if first {
                piece.trim_start_matches(is_whitespace)
            } else {
                piece
            }
        }
        out.clear();
        let mut pos = 0;
        let mut first = true;
        if !self.masks.is_empty() {
            for (start, end, index) in self.masked_ranges(content) {
                self.push_text(out, lead(&content[pos..start], first), false);
                first = false;
                out.push(MASK_BYTE);
                out.push(index);
                pos = end;
            }
        }
        self.push_text(out, lead(&content[pos..], first), true);
    }

    /// True when no option normalizes lines: whitespace mode `none`, no case
    /// folding and no masks. Two lines then have equal keys exactly when their
    /// bytes, terminators included, are equal (content never holds CR or LF,
    /// so the raw bytes determine content and terminator).
    pub fn is_exact(&self) -> bool {
        self.whitespace == WhitespaceMode::None && self.fold.is_none() && self.masks.is_empty()
    }

    /// The key of one line, as bytes. Distinct terminators give distinct keys
    /// only under whitespace mode `none`.
    pub fn key(&self, content: &str, terminator: Terminator) -> Vec<u8> {
        let mut out = Vec::with_capacity(content.len() + 2);
        self.key_into(content, terminator, &mut out);
        out
    }

    /// Like [`KeyBuilder::key`], writing into `out` (cleared first), so one
    /// buffer serves every line.
    pub fn key_into(&self, content: &str, terminator: Terminator, out: &mut Vec<u8>) {
        out.clear();
        let mut pos = 0;
        if !self.masks.is_empty() {
            for (start, end, index) in self.masked_ranges(content) {
                self.push_text(out, &content[pos..start], false);
                out.push(MASK_BYTE);
                out.push(index);
                pos = end;
            }
        }
        self.push_text(out, &content[pos..], true);
        if self.whitespace == WhitespaceMode::None {
            // 0xFF never occurs in UTF-8, so the suffix cannot collide with content.
            out.push(0xFF);
            out.push(match terminator {
                Terminator::Lf => b'n',
                Terminator::CrLf => b'w',
                Terminator::Cr => b'r',
                Terminator::None => b'0',
            });
        }
    }

    /// Appends one unmasked piece of a line. A placeholder is not whitespace:
    /// runs never cross it, and only the piece at the end of the line loses
    /// its trailing whitespace.
    ///
    /// One pass, straight into `out`. The whitespace characters are ASCII
    /// bytes, which never occur inside a multi-byte UTF-8 sequence, so the
    /// text is split at byte positions that are always character boundaries.
    fn push_text(&self, out: &mut Vec<u8>, text: &str, at_end: bool) {
        let text = if at_end {
            match self.whitespace {
                WhitespaceMode::Trailing | WhitespaceMode::Change => {
                    text.trim_end_matches(is_whitespace)
                }
                _ => text,
            }
        } else {
            text
        };
        let collapse = match self.whitespace {
            WhitespaceMode::None | WhitespaceMode::Trailing => {
                self.push_folded(out, text);
                return;
            }
            WhitespaceMode::Change => true,
            WhitespaceMode::All => false,
        };
        let bytes = text.as_bytes();
        if bytes.is_ascii() {
            push_ascii_runs(
                out,
                bytes,
                collapse,
                self.fold.is_some(),
                &self.double_space,
            );
            return;
        }
        let (mut start, mut i) = (0, 0);
        while i < bytes.len() {
            if is_whitespace_byte(bytes[i]) {
                self.push_folded(out, &text[start..i]);
                while i < bytes.len() && is_whitespace_byte(bytes[i]) {
                    i += 1;
                }
                if collapse {
                    out.push(b' ');
                }
                start = i;
            } else {
                i += 1;
            }
        }
        self.push_folded(out, &text[start..]);
    }

    /// Appends `text`, case folded when the options ask for it.
    #[inline]
    fn push_folded(&self, out: &mut Vec<u8>, text: &str) {
        match self.fold {
            Some(fold) => fold_into(fold, text, out),
            None => out.extend_from_slice(text.as_bytes()),
        }
    }
}

/// True for the bytes of the whitespace characters ([`is_whitespace`]).
#[inline]
fn is_whitespace_byte(b: u8) -> bool {
    matches!(b, b'\t' | 0x0B | 0x0C | b' ')
}

/// Whitespace modes `change` (`collapse`: each run becomes one space) and
/// `all` (runs are dropped) on ASCII text, lowercased with `fold` (the
/// simple case folding of ASCII).
///
/// The text is copied and lowercased in bulk first (lowercasing leaves
/// whitespace alone), then its runs are closed up in place; the result is
/// never longer than the text. Under `change`, text whose only whitespace is
/// spaces (the common case) is searched for two spaces in a row
/// (`double_space`), and each piece after such a run moves once; other text
/// is compacted byte by byte, without branches.
fn push_ascii_runs(
    out: &mut Vec<u8>,
    bytes: &[u8],
    collapse: bool,
    fold: bool,
    double_space: &memchr::memmem::Finder<'_>,
) {
    let at = out.len();
    out.extend_from_slice(bytes);
    if fold {
        out[at..].make_ascii_lowercase();
    }
    let dst = &mut out[at..];
    if !collapse || memchr::memchr3(b'\t', 0x0B, 0x0C, bytes).is_some() {
        let n = compact_runs(dst, collapse);
        out.truncate(at + n);
        return;
    }
    let (mut write, mut read) = (0, 0);
    // `read` is 0 or just past a run, so the next two spaces in a row start
    // the next run of two or more.
    while let Some(found) = double_space.find(&bytes[read..]) {
        let start = read + found;
        let mut end = start + 2;
        while bytes.get(end) == Some(&b' ') {
            end += 1;
        }
        // Keep `read..=start` (the piece and one space), drop the rest.
        if write != read {
            dst.copy_within(read..=start, write);
        }
        write += start + 1 - read;
        read = end;
    }
    if write != read {
        dst.copy_within(read.., write);
    }
    write += bytes.len() - read;
    out.truncate(at + write);
}

/// Closes up the whitespace runs of `text` in place: one space per run with
/// `collapse`, none without. Returns the new length.
fn compact_runs(text: &mut [u8], collapse: bool) -> usize {
    let mut n = 0;
    let mut in_run = false;
    for i in 0..text.len() {
        let b = text[i];
        let ws = is_whitespace_byte(b);
        text[n] = if ws { b' ' } else { b };
        n += usize::from(!ws || (collapse && !in_run));
        in_run = ws;
    }
    n
}

/// Appends the simple case folding of `text`. ASCII runs are lowercased as
/// bytes: the only ASCII entries of CaseFolding.txt map `A`–`Z` to `a`–`z`.
/// Other characters go through the Unicode tables. A run of non-ASCII bytes
/// starts and ends next to an ASCII byte (or the text's ends), so it is
/// always a whole number of characters.
fn fold_into(fold: CaseMapperBorrowed<'static>, text: &str, out: &mut Vec<u8>) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii() {
            i += 1;
        }
        if i > start {
            let at = out.len();
            out.extend_from_slice(&bytes[start..i]);
            out[at..].make_ascii_lowercase();
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii() {
            i += 1;
        }
        if i > start {
            let mut buf = [0; 4];
            for c in text[start..i].chars() {
                out.extend_from_slice(fold.simple_fold(c).encode_utf8(&mut buf).as_bytes());
            }
        }
    }
}

/// The masked parts of `content` as `(start, end, pattern)` byte ranges,
/// sorted and disjoint (spec §7.2 step 1).
///
/// Each pattern's non-empty matches are found over the whole content,
/// leftmost-first and without overlaps. Patterns are applied in order: a
/// match is kept unless it overlaps a match kept for an earlier pattern.
pub fn masked_ranges(masks: &[Regex], content: &str) -> Vec<(usize, usize, u8)> {
    let mut kept: Vec<(usize, usize, u8)> = Vec::new();
    for (index, mask) in masks.iter().enumerate() {
        let index = u8::try_from(index).unwrap_or(u8::MAX);
        let mut merged = Vec::with_capacity(kept.len());
        let mut k = 0;
        for m in mask.find_iter(content).filter(|m| !m.is_empty()) {
            while k < kept.len() && kept[k].1 <= m.start() {
                merged.push(kept[k]);
                k += 1;
            }
            // kept[k] is the first earlier match ending after m starts.
            if k < kept.len() && kept[k].0 < m.end() {
                continue;
            }
            merged.push((m.start(), m.end(), index));
        }
        merged.extend_from_slice(&kept[k..]);
        kept = merged;
    }
    kept
}

/// Replaces every run of whitespace with one U+0020.
fn collapse_runs(text: &str) -> std::borrow::Cow<'_, str> {
    let mut out = String::with_capacity(text.len());
    let mut in_run = false;
    for c in text.chars() {
        if is_whitespace(c) {
            if !in_run {
                out.push(' ');
            }
            in_run = true;
        } else {
            out.push(c);
            in_run = false;
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Applies a whitespace mode to text (no terminator handling).
pub fn normalize_whitespace(content: &str, mode: WhitespaceMode) -> std::borrow::Cow<'_, str> {
    match mode {
        WhitespaceMode::None => std::borrow::Cow::Borrowed(content),
        WhitespaceMode::Trailing => {
            std::borrow::Cow::Borrowed(content.trim_end_matches(is_whitespace))
        }
        WhitespaceMode::Change => std::borrow::Cow::Owned(
            collapse_runs(content.trim_end_matches(is_whitespace)).into_owned(),
        ),
        WhitespaceMode::All => {
            std::borrow::Cow::Owned(content.chars().filter(|c| !is_whitespace(*c)).collect())
        }
    }
}

/// Maps keys to dense `u32` token IDs shared by both sides, in first-seen order.
#[derive(Debug, Default)]
pub struct Interner {
    ids: HashMap<Vec<u8>, u32>,
}

impl Interner {
    /// An empty interner.
    pub fn new() -> Interner {
        Interner::default()
    }

    /// The ID of `key`, assigning the next one if it is new.
    pub fn intern(&mut self, key: Vec<u8>) -> u32 {
        let next = u32::try_from(self.ids.len()).unwrap_or(u32::MAX);
        *self.ids.entry(key).or_insert(next)
    }

    /// Number of distinct keys.
    pub fn num_tokens(&self) -> u32 {
        u32::try_from(self.ids.len()).unwrap_or(u32::MAX)
    }
}

/// Interns the keys of both sides' lines. Returns the token sequences.
pub fn intern_sides(
    builder: &KeyBuilder,
    interner: &mut Interner,
    left_text: &str,
    left: &[Line],
    right_text: &str,
    right: &[Line],
) -> (Vec<u32>, Vec<u32>) {
    let mut tokens = |text: &str, lines: &[Line]| -> Vec<u32> {
        lines
            .iter()
            .map(|l| interner.intern(builder.key(l.content(text), l.terminator)))
            .collect()
    };
    let l = tokens(left_text, left);
    let r = tokens(right_text, right);
    (l, r)
}

// ---------------------------------------------------------------------------
// Large-input path: trim, then intern only the lines in between.
//
// `tokenize_core` gives the same line diff as `intern_sides` followed by
// `line_diff::align`, and `is_approximate_core` the same answer as
// `line_diff::is_approximate` on the full token sequences:
//
// - The common prefix and suffix are the ones `align` trims (equal keys, the
//   prefix first), found on raw bytes first and then extended by keys.
// - Only the lines in between (the core) are interned. Token ids are a
//   relabeling of the full-sequence ids, which the algorithms do not depend on.
// - No key is stored. Each token remembers the first line that has it; two
//   lines with the same raw bytes have the same key, and other lines with the
//   same hash build both keys again to compare them. When no option
//   normalizes lines (`KeyBuilder::is_exact`) the key is the raw bytes. The
//   hash table holds 8 bytes per distinct key, plus 4 for its first line.
// - Both sides' keys are built and hashed on two threads (where there are
//   threads), then interned in order.
// ---------------------------------------------------------------------------

/// One side of a comparison: its text and line index.
#[derive(Debug, Clone, Copy)]
pub struct TextLines<'a> {
    /// The text.
    pub text: &'a str,
    /// Its lines.
    pub lines: &'a LineIndex,
}

impl<'a> TextLines<'a> {
    /// Line `i` with its terminator, as raw bytes.
    #[inline]
    pub fn raw(&self, i: usize) -> &'a [u8] {
        &self.text.as_bytes()[self.lines.raw_range(i)]
    }

    /// Line `i`.
    #[inline]
    pub fn line(&self, i: usize) -> Line {
        self.lines.line(self.text, i)
    }

    /// Number of lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// True when there are no lines.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// Tokens of the lines between the common prefix and suffix of two sides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTokens {
    /// Lines of the common prefix (equal keys, line by line).
    pub prefix: usize,
    /// Lines of the common suffix, after the prefix.
    pub suffix: usize,
    /// Tokens of left lines `prefix..left_len - suffix`.
    pub left: Vec<u32>,
    /// Tokens of right lines `prefix..right_len - suffix`.
    pub right: Vec<u32>,
    /// Number of distinct tokens.
    pub num_tokens: u32,
}

/// Length of the common prefix of two byte strings.
fn common_prefix_bytes(a: &[u8], b: &[u8]) -> usize {
    const CHUNK: usize = 4096;
    let n = a.len().min(b.len());
    let mut i = 0;
    while i + CHUNK <= n && a[i..i + CHUNK] == b[i..i + CHUNK] {
        i += CHUNK;
    }
    i + a[i..n]
        .iter()
        .zip(&b[i..n])
        .take_while(|(x, y)| x == y)
        .count()
}

/// Length of the common suffix of two byte strings, at most `max`.
fn common_suffix_bytes(a: &[u8], b: &[u8], max: usize) -> usize {
    const CHUNK: usize = 4096;
    let n = a.len().min(b.len()).min(max);
    let (a, b) = (&a[a.len() - n..], &b[b.len() - n..]);
    let mut q = 0;
    while q + CHUNK <= n && a[n - q - CHUNK..n - q] == b[n - q - CHUNK..n - q] {
        q += CHUNK;
    }
    q + a[..n - q]
        .iter()
        .rev()
        .zip(b[..n - q].iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

/// First line index in `lo..hi` whose value of `f` is false, where `f` is
/// true then false over the range.
fn partition(lo: usize, hi: usize, f: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (lo, hi);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if f(mid) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/// The common prefix and suffix in lines: the ones `line_diff::align` trims
/// from the full token sequences.
pub fn common_lines(
    builder: &KeyBuilder,
    left: TextLines<'_>,
    right: TextLines<'_>,
) -> (usize, usize) {
    let (lbytes, rbytes) = (left.text.as_bytes(), right.text.as_bytes());
    let (nl, nr) = (left.len(), right.len());
    let both = nl.min(nr);
    let keys_equal = |i: usize, j: usize, buf: &mut (Vec<u8>, Vec<u8>)| {
        let (ll, rl) = (left.line(i), right.line(j));
        builder.key_into(ll.content(left.text), ll.terminator, &mut buf.0);
        builder.key_into(rl.content(right.text), rl.terminator, &mut buf.1);
        buf.0 == buf.1
    };
    let mut buf = (Vec::new(), Vec::new());

    // Prefix: the lines that end within the common bytes are equal, except
    // possibly the last one (a lone CR there depends on the next byte).
    let common_head = common_prefix_bytes(lbytes, rbytes);
    let mut prefix = partition(0, both, |i| left.lines.raw_range(i).end <= common_head);
    while prefix > 0 && left.raw(prefix - 1) != right.raw(prefix - 1) {
        prefix -= 1;
    }
    if !builder.is_exact() {
        while prefix < both && keys_equal(prefix, prefix, &mut buf) {
            prefix += 1;
        }
    }

    // Suffix, after the prefix: lines that start within the common bytes are
    // equal, except possibly the first one.
    let max_lines = both - prefix;
    let tail_l = lbytes.len() - left.lines.start(prefix);
    let tail_r = rbytes.len() - right.lines.start(prefix);
    let common_tail = common_suffix_bytes(lbytes, rbytes, tail_l.min(tail_r));
    let first_l = partition(prefix, nl, |i| {
        left.lines.start(i) < lbytes.len() - common_tail
    });
    let first_r = partition(prefix, nr, |j| {
        right.lines.start(j) < rbytes.len() - common_tail
    });
    let mut suffix = (nl - first_l).min(nr - first_r).min(max_lines);
    while suffix > 0 && left.raw(nl - suffix) != right.raw(nr - suffix) {
        suffix -= 1;
    }
    if !builder.is_exact() {
        while suffix < max_lines && keys_equal(nl - 1 - suffix, nr - 1 - suffix, &mut buf) {
            suffix += 1;
        }
    }
    (prefix, suffix)
}

/// The table hash of a key: 32 bits, stored next to the token so the table
/// can grow without reading keys again.
#[inline]
fn hash32(state: foldhash::fast::FixedState, key: &[u8]) -> u32 {
    use std::hash::BuildHasher;
    // The high half of the 64-bit hash; truncation is the point.
    #[allow(clippy::cast_possible_truncation)]
    let h = (state.hash_one(key) >> 32) as u32;
    h
}

/// Spreads a 32-bit hash over the 64 bits the table uses (bucket bits from
/// the low half, tag bits from the high half).
#[inline]
fn spread(h: u32) -> u64 {
    u64::from(h) | (u64::from(h.wrapping_mul(0x9E37_79B9)) << 32)
}

/// Fixed seed: the hash never affects output, only speed.
const HASH_SEED: u64 = 0x6469_6666_796d_616e;

#[derive(Debug, Clone, Copy)]
struct Slot {
    token: u32,
    hash: u32,
}

/// The keys of the tokens: token `t` is the key of global line `first[t]`
/// (left lines first, then right lines; both sides together have fewer than
/// 2^32 lines). No key is stored: two lines with the same raw bytes have the
/// same key, and other pairs build both keys again (only when their hashes
/// are equal, so almost only for lines whose keys are equal).
struct LineKeys<'a> {
    sides: [TextLines<'a>; 2],
    first: Vec<u32>,
    /// The key rules; `None` when no option normalizes lines, so keys are
    /// the raw bytes.
    builder: Option<&'a KeyBuilder>,
    /// Scratch keys of the two lines compared.
    bufs: (Vec<u8>, Vec<u8>),
}

impl<'a> LineKeys<'a> {
    /// The raw bytes (with terminator) of global line `line`.
    #[inline]
    fn raw(&self, line: u32) -> &'a [u8] {
        let line = line as usize;
        let left_len = self.sides[0].len();
        if line < left_len {
            self.sides[0].raw(line)
        } else {
            self.sides[1].raw(line - left_len)
        }
    }

    /// Writes the key of global line `line` into `out`.
    fn key_into(sides: &[TextLines<'_>; 2], builder: &KeyBuilder, line: u32, out: &mut Vec<u8>) {
        let line = line as usize;
        let left_len = sides[0].len();
        let (side, i) = if line < left_len {
            (sides[0], line)
        } else {
            (sides[1], line - left_len)
        };
        let l = side.line(i);
        builder.key_into(l.content(side.text), l.terminator, out);
    }

    /// True when token `token` has the key of global line `line`, whose raw
    /// bytes are `raw`.
    #[inline]
    fn same(&mut self, token: u32, line: u32, raw: &[u8]) -> bool {
        let stored = self.first[token as usize];
        self.raw(stored) == raw || (self.builder.is_some() && self.same_key(stored, line))
    }

    /// True when global lines `a` and `b` have the same normalized key.
    #[inline(never)]
    fn same_key(&mut self, a: u32, b: u32) -> bool {
        let Some(builder) = self.builder else {
            return false;
        };
        let (ka, kb) = &mut self.bufs;
        LineKeys::key_into(&self.sides, builder, a, ka);
        LineKeys::key_into(&self.sides, builder, b, kb);
        ka == kb
    }

    fn len(&self) -> usize {
        self.first.len()
    }
}

/// Interns line keys of two sides into dense tokens, in first-seen order.
struct LineInterner<'a> {
    table: hashbrown::HashTable<Slot>,
    state: foldhash::fast::FixedState,
    keys: LineKeys<'a>,
}

impl<'a> LineInterner<'a> {
    fn new(builder: &'a KeyBuilder, sides: [TextLines<'a>; 2]) -> LineInterner<'a> {
        LineInterner {
            table: hashbrown::HashTable::new(),
            state: foldhash::fast::FixedState::with_seed(HASH_SEED),
            keys: LineKeys {
                sides,
                first: Vec::new(),
                builder: (!builder.is_exact()).then_some(builder),
                bufs: (Vec::new(), Vec::new()),
            },
        }
    }

    /// The token of the key of global line `line` (hashed with
    /// [`line_hash`]), if it was interned.
    fn find(&mut self, h: u32, line: u32) -> Option<u32> {
        let LineInterner { table, keys, .. } = self;
        let raw = keys.raw(line);
        table
            .find(spread(h), |s| s.hash == h && keys.same(s.token, line, raw))
            .map(|s| s.token)
    }

    /// The token of the key of global line `line`, whose [`line_hash`] is
    /// `h`, assigning the next one if it is new.
    #[inline]
    fn intern_hashed(&mut self, h: u32, line: u32) -> u32 {
        let LineInterner { table, keys, .. } = self;
        let raw = keys.raw(line);
        match table.entry(
            spread(h),
            |s| s.hash == h && keys.same(s.token, line, raw),
            |s| spread(s.hash),
        ) {
            hashbrown::hash_table::Entry::Occupied(e) => e.get().token,
            hashbrown::hash_table::Entry::Vacant(e) => {
                let token = u32::try_from(keys.len()).unwrap_or(u32::MAX);
                keys.first.push(line);
                e.insert(Slot { token, hash: h });
                token
            }
        }
    }

    fn num_tokens(&self) -> u32 {
        u32::try_from(self.keys.len()).unwrap_or(u32::MAX)
    }
}

/// [`hash32`] of the key of line `i` of `lines`: of the raw bytes when no
/// option normalizes lines, otherwise of the key built into `buf`.
#[inline]
fn line_hash(
    state: foldhash::fast::FixedState,
    builder: &KeyBuilder,
    lines: TextLines<'_>,
    i: usize,
    buf: &mut Vec<u8>,
) -> u32 {
    if builder.is_exact() {
        hash32(state, lines.raw(i))
    } else {
        let l = lines.line(i);
        builder.key_into(l.content(lines.text), l.terminator, buf);
        hash32(state, buf)
    }
}

/// [`line_hash`] of each core line of both sides. The sides are hashed (and
/// their keys built) on two threads when they are large and the platform
/// has threads; the values do not depend on that.
fn core_hashes(
    state: foldhash::fast::FixedState,
    builder: &KeyBuilder,
    left: TextLines<'_>,
    right: TextLines<'_>,
    prefix: usize,
    suffix: usize,
) -> (Vec<u32>, Vec<u32>) {
    /// Fewer core lines than this per side are hashed on one thread.
    #[cfg(not(target_family = "wasm"))]
    const PARALLEL_MIN_LINES: usize = 1 << 16;
    let hashes = |lines: TextLines<'_>| -> Vec<u32> {
        let core = prefix..lines.len() - suffix;
        if builder.is_exact() {
            return core.map(|i| hash32(state, lines.raw(i))).collect();
        }
        let mut buf = Vec::new();
        core.map(|i| line_hash(state, builder, lines, i, &mut buf))
            .collect()
    };
    #[cfg(not(target_family = "wasm"))]
    if (left.len() - prefix - suffix).min(right.len() - prefix - suffix) >= PARALLEL_MIN_LINES {
        let pair = std::thread::scope(|scope| {
            let handle = std::thread::Builder::new()
                .name("diffyman-hash".to_owned())
                .spawn_scoped(scope, || hashes(right));
            let l = hashes(left);
            (l, handle.ok().and_then(|h| h.join().ok()))
        });
        if let (l, Some(r)) = pair {
            return (l, r);
        }
    }
    (hashes(left), hashes(right))
}

/// Global number of line `i` of `side` (0 left, 1 right; left lines first).
fn global_line(left_len: usize, side: usize, i: usize) -> u32 {
    let n = if side == 0 { i } else { left_len + i };
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Trims the common prefix and suffix and interns the lines in between.
///
/// `line_diff::align` over [`CoreTokens::left`] and [`CoreTokens::right`]
/// gives the alignment of those lines; every prefix and suffix line is
/// matched to the same line on the other side.
pub fn tokenize_core(
    builder: &KeyBuilder,
    left: TextLines<'_>,
    right: TextLines<'_>,
) -> CoreTokens {
    let (prefix, suffix) = common_lines(builder, left, right);
    let mut interner = LineInterner::new(builder, [left, right]);
    // Hash both sides first (in parallel where there are threads), then
    // intern in order. Each hash is overwritten by its token, so the hashes
    // need no memory of their own.
    let (mut l, mut r) = core_hashes(interner.state, builder, left, right, prefix, suffix);
    for (side, tokens) in [(0, &mut l), (1, &mut r)] {
        for (k, slot) in tokens.iter_mut().enumerate() {
            *slot = interner.intern_hashed(*slot, global_line(left.len(), side, prefix + k));
        }
    }
    CoreTokens {
        prefix,
        suffix,
        left: l,
        right: r,
        num_tokens: interner.num_tokens(),
    }
}

/// One side of a trimmed comparison, for [`crate::slider`]: two core lines
/// compare by token, any other pair by key (raw bytes when no option
/// normalizes lines), so a run can slide into the common prefix or suffix.
pub struct KeyedSide<'a> {
    builder: &'a KeyBuilder,
    lines: TextLines<'a>,
    prefix: usize,
    tokens: &'a [u32],
    bufs: (Vec<u8>, Vec<u8>),
}

impl<'a> KeyedSide<'a> {
    /// `tokens` are the core tokens of `lines`, which start at line `prefix`.
    pub fn new(
        builder: &'a KeyBuilder,
        lines: TextLines<'a>,
        prefix: usize,
        tokens: &'a [u32],
    ) -> KeyedSide<'a> {
        KeyedSide {
            builder,
            lines,
            prefix,
            tokens,
            bufs: (Vec::new(), Vec::new()),
        }
    }

    fn token(&self, i: usize) -> Option<u32> {
        i.checked_sub(self.prefix)
            .and_then(|k| self.tokens.get(k))
            .copied()
    }
}

impl crate::slider::SliderSide for KeyedSide<'_> {
    fn len(&self) -> usize {
        self.lines.len()
    }

    fn same(&mut self, i: usize, j: usize) -> (bool, u64) {
        if let (Some(a), Some(b)) = (self.token(i), self.token(j)) {
            return (a == b, 1);
        }
        let (a, b) = (self.lines.raw(i), self.lines.raw(j));
        let cost = 1 + (a.len() as u64 + b.len() as u64) / 64;
        if self.builder.is_exact() {
            return (a == b, cost);
        }
        let (li, lj) = (self.lines.line(i), self.lines.line(j));
        let text = self.lines.text;
        self.builder
            .key_into(li.content(text), li.terminator, &mut self.bufs.0);
        self.builder
            .key_into(lj.content(text), lj.terminator, &mut self.bufs.1);
        (self.bufs.0 == self.bufs.1, cost)
    }

    fn content(&self, i: usize) -> &[u8] {
        self.lines.line(i).content(self.lines.text).as_bytes()
    }
}

/// The full-file alignment of a trimmed comparison (`core_alignment` is
/// `align` over the core), compacted by [`crate::slider::compact`] with
/// [`crate::slider::compaction_budget`].
pub fn compact_core(
    builder: &KeyBuilder,
    left: TextLines<'_>,
    right: TextLines<'_>,
    core: &CoreTokens,
    core_alignment: &crate::line_diff::Alignment,
) -> (crate::line_diff::Alignment, crate::slider::CompactReport) {
    let mut full = crate::ops::expand_trimmed(
        core_alignment,
        core.prefix,
        core.suffix,
        left.len(),
        right.len(),
    );
    let report = crate::slider::compact(
        &mut full,
        &mut KeyedSide::new(builder, left, core.prefix, &core.left),
        &mut KeyedSide::new(builder, right, core.prefix, &core.right),
        crate::slider::compaction_budget(left.len(), right.len()),
    );
    (full, report)
}

/// [`crate::line_diff::is_approximate`] of the full token sequences, from the
/// core tokens and the core alignment (`align` over the core).
///
/// A changed line counts when its key also occurs on the other side. Prefix
/// and suffix lines have the same keys on both sides, so a key "occurs on the
/// other side" when it is in the other core or in the common lines. The
/// common lines are only read (hashed) when the core alone cannot decide.
/// A core with one side empty (a pure insertion or deletion) is never
/// approximate: no search runs on it.
pub fn is_approximate_core(
    builder: &KeyBuilder,
    left: TextLines<'_>,
    right: TextLines<'_>,
    core: &CoreTokens,
    alignment: &crate::line_diff::Alignment,
) -> bool {
    if core.left.is_empty() || core.right.is_empty() {
        return false;
    }
    let max = crate::line_diff::EXACT_MAX_EDITS;
    let size = core.num_tokens as usize;
    let presence = |side: &[u32]| -> Vec<bool> {
        let mut seen = vec![false; size];
        for &t in side {
            seen[t as usize] = true;
        }
        seen
    };
    let (in_left, in_right) = (presence(&core.left), presence(&core.right));
    // (side, core index) of changed lines whose key the other core lacks.
    let mut counted = 0usize;
    let mut pending: Vec<(usize, usize)> = Vec::new();
    for (i, (t, removed)) in core.left.iter().zip(&alignment.removed).enumerate() {
        if *removed {
            if in_right[*t as usize] {
                counted += 1;
            } else {
                pending.push((0, i));
            }
        }
    }
    for (j, (t, added)) in core.right.iter().zip(&alignment.added).enumerate() {
        if *added {
            if in_left[*t as usize] {
                counted += 1;
            } else {
                pending.push((1, j));
            }
        }
    }
    if counted > max {
        return true;
    }
    if counted + pending.len() <= max {
        return false;
    }
    drop((in_left, in_right));

    // Weight of each pending token: how many pending lines carry it.
    let mut weight: Vec<u32> = vec![0; size];
    for &(side, i) in &pending {
        let t = if side == 0 {
            core.left[i]
        } else {
            core.right[i]
        };
        weight[t as usize] += 1;
    }
    // Index the keys of the pending tokens, then look up every common line.
    let sides = [left, right];
    let mut interner = LineInterner::new(builder, sides);
    let state = interner.state;
    let mut tokens_of: Vec<u32> = Vec::new();
    let mut buf = Vec::new();
    for &(side, i) in &pending {
        let line = core.prefix + i;
        let h = line_hash(state, builder, sides[side], line, &mut buf);
        let local = interner.intern_hashed(h, global_line(left.len(), side, line));
        if local as usize == tokens_of.len() {
            tokens_of.push(if side == 0 {
                core.left[i]
            } else {
                core.right[i]
            });
        }
    }
    let n = left.len();
    let common = (0..core.prefix).chain(n - core.suffix..n);
    for i in common {
        let h = line_hash(state, builder, left, i, &mut buf);
        if let Some(local) = interner.find(h, global_line(left.len(), 0, i)) {
            let t = tokens_of[local as usize] as usize;
            counted += weight[t] as usize;
            weight[t] = 0;
            if counted > max {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(opts: &KeyOptions, content: &str, t: Terminator) -> Vec<u8> {
        KeyBuilder::new(opts).unwrap().key(content, t)
    }

    #[test]
    fn none_mode_includes_terminator() {
        let o = KeyOptions::default();
        assert_ne!(key(&o, "a", Terminator::Lf), key(&o, "a", Terminator::CrLf));
        assert_ne!(key(&o, "a", Terminator::Lf), key(&o, "a", Terminator::None));
        assert_eq!(key(&o, "a", Terminator::Lf), key(&o, "a", Terminator::Lf));
        assert_ne!(key(&o, "a ", Terminator::Lf), key(&o, "a", Terminator::Lf));
    }

    #[test]
    fn whitespace_modes() {
        let o = |whitespace| KeyOptions {
            whitespace,
            ..KeyOptions::default()
        };
        let t = o(WhitespaceMode::Trailing);
        assert_eq!(
            key(&t, "a b \t", Terminator::Lf),
            key(&t, "a b", Terminator::None)
        );
        assert_ne!(
            key(&t, "a  b", Terminator::Lf),
            key(&t, "a b", Terminator::Lf)
        );
        let c = o(WhitespaceMode::Change);
        assert_eq!(
            key(&c, "a \t b  ", Terminator::CrLf),
            key(&c, "a b", Terminator::Lf)
        );
        assert_ne!(
            key(&c, "ab", Terminator::Lf),
            key(&c, "a b", Terminator::Lf)
        );
        assert_eq!(
            key(&c, "  x", Terminator::Lf),
            key(&c, "\tx", Terminator::Lf)
        );
        let a = o(WhitespaceMode::All);
        assert_eq!(
            key(&a, " a\u{b}b\u{c} ", Terminator::Lf),
            key(&a, "ab", Terminator::Cr)
        );
        // U+00A0 is not whitespace for comparison.
        assert_ne!(
            key(&a, "a\u{a0}b", Terminator::Lf),
            key(&a, "ab", Terminator::Lf)
        );
    }

    #[test]
    fn case_folding_is_simple() {
        let o = KeyOptions {
            ignore_case: true,
            ..KeyOptions::default()
        };
        assert_eq!(
            key(&o, "HeLLo Σ", Terminator::Lf),
            key(&o, "hello σ", Terminator::Lf)
        );
        assert_eq!(key(&o, "ς", Terminator::Lf), key(&o, "σ", Terminator::Lf));
        // Simple folding keeps ß as is (full folding would give "ss").
        assert_ne!(key(&o, "ß", Terminator::Lf), key(&o, "ss", Terminator::Lf));
    }

    fn masked(patterns: &[&str], whitespace: WhitespaceMode) -> KeyOptions {
        KeyOptions {
            whitespace,
            regex: patterns.iter().map(|p| (*p).to_owned()).collect(),
            ..KeyOptions::default()
        }
    }

    fn errors(patterns: &[String]) -> MaskErrors {
        compile_masks(patterns).expect_err("patterns must be rejected")
    }

    #[test]
    fn regex_masks_use_placeholders() {
        let o = masked(&[r"\d+", "x"], WhitespaceMode::None);
        let k = |s: &str| key(&o, s, Terminator::Lf);
        assert_eq!(k("id 123"), k("id 9"));
        // A masked value is not the same as no value.
        assert_ne!(k("id 123"), k("id "));
        // Each pattern has its own placeholder.
        assert_ne!(k("id 7"), k("id x"));
        assert_eq!(k("7x"), k("12x"));
        // Unmasked text still counts, and so does the terminator under `none`.
        assert_ne!(k("id 1 a"), k("id 1 b"));
        assert_ne!(k("id 1"), key(&o, "id 2", Terminator::CrLf));
        // No masks: the key is unchanged.
        let plain = KeyOptions::default();
        assert_eq!(
            key(&masked(&[], WhitespaceMode::None), "a 1", Terminator::Lf),
            key(&plain, "a 1", Terminator::Lf)
        );
    }

    #[test]
    fn earlier_patterns_win_overlaps_and_empty_matches_mask_nothing() {
        let b =
            |patterns: &[&str]| KeyBuilder::new(&masked(patterns, WhitespaceMode::None)).unwrap();
        assert_eq!(b(&["ab", "bc"]).masked_ranges("abc"), [(0, 2, 0)]);
        assert_eq!(b(&["bc", "ab"]).masked_ranges("abc"), [(1, 3, 0)]);
        assert_eq!(
            b(&["b", "a|c"]).masked_ranges("abcb"),
            [(0, 1, 1), (1, 2, 0), (2, 3, 1), (3, 4, 0)]
        );
        assert!(b(&["x*"]).masked_ranges("abc").is_empty());
        assert_eq!(
            b(&["x*"]).key("abc", Terminator::Lf),
            b(&[]).key("abc", Terminator::Lf)
        );
        // Anchors refer to the whole line content.
        assert_eq!(b(&[r"^\d+"]).masked_ranges("12 ab 34"), [(0, 2, 0)]);
        assert_eq!(b(&[r"\d+$"]).masked_ranges("12 ab 34"), [(6, 8, 0)]);
    }

    #[test]
    fn whitespace_rules_treat_placeholders_as_text() {
        let pat = [r"\d+"];
        let t = masked(&pat, WhitespaceMode::Trailing);
        let c = masked(&pat, WhitespaceMode::Change);
        let a = masked(&pat, WhitespaceMode::All);
        let lf = Terminator::Lf;
        // Whitespace before a placeholder is not trailing.
        assert_ne!(key(&t, "a  1", lf), key(&t, "a 1", lf));
        assert_eq!(key(&t, "a 1 \t", lf), key(&t, "a 2", Terminator::None));
        // Runs collapse on each side of a placeholder but never across it.
        assert_eq!(key(&c, "a  1\t b ", lf), key(&c, "a 2 b", lf));
        assert_ne!(key(&c, "a1", lf), key(&c, "a 1", lf));
        assert_eq!(key(&a, " a 1 b", lf), key(&a, "a2b", lf));
        // Case folding applies to the unmasked text.
        let f = KeyOptions {
            ignore_case: true,
            ..masked(&pat, WhitespaceMode::None)
        };
        assert_eq!(key(&f, "ID 1", lf), key(&f, "id 22", lf));
    }

    #[test]
    fn invalid_and_oversized_patterns_are_rejected() {
        let one = |p: &str| vec![p.to_owned()];
        let invalid = MaskErrors {
            invalid: true,
            limit: false,
        };
        let limit = MaskErrors {
            invalid: false,
            limit: true,
        };
        assert_eq!(errors(&one("(")), invalid);
        assert_eq!(errors(&one(r"\p{NoSuchClass}")), invalid);
        assert_eq!(errors(&one("a{2,1}")), invalid);
        // Look-around and backreferences are not part of the dialect.
        assert_eq!(errors(&one("(?=a)")), invalid);
        assert_eq!(errors(&one(r"(a)\1")), invalid);
        // Patterns that could match invalid UTF-8 are rejected.
        assert_eq!(errors(&one(r"(?-u:\xFF)")), invalid);
        assert_eq!(errors(&one(r"\w{100}")), limit);
        // The example of spec §7.2.1.
        assert!(compile_masks(&one(r"\w{31}")).is_ok());
        assert_eq!(errors(&one(r"\w{32}")), limit);
        assert_eq!(
            errors(&one(&"a".repeat(MAX_REGEX_PATTERN_CHARS + 1))),
            limit
        );
        assert!(compile_masks(&one(&"é".repeat(MAX_REGEX_PATTERN_CHARS))).is_ok());
        let many = vec!["a".to_owned(); MAX_REGEX_PATTERNS + 1];
        assert_eq!(errors(&many), limit);
        assert!(compile_masks(&many[1..]).is_ok());
        assert_eq!(
            errors(&[r"\d".to_owned(), "(".to_owned(), r"\w{100}".to_owned()]),
            MaskErrors {
                invalid: true,
                limit: true
            }
        );
        assert!(compile_masks(&[]).unwrap().is_empty());
        assert!(compile_masks(&one("")).is_ok());
    }

    /// The unit cap, not the byte limit, decides: the largest accepted
    /// repetition of each costly building block still compiles with a quarter
    /// of [`REGEX_SIZE_LIMIT`] here, which leaves room for the different
    /// pointer sizes of other platforms.
    #[test]
    fn unit_cap_decides_before_byte_limits() {
        let units = |p: &str| {
            let nfa = thompson::Compiler::new()
                .configure(thompson::Config::new().nfa_size_limit(Some(REGEX_SIZE_LIMIT)))
                .build(p)
                .ok()?;
            Some(nfa_units(&nfa))
        };
        for atom in [
            r"\w",
            r"(?i)\pL",
            r"[^\n]",
            r"(?-u:[\x00\x02\x04\x06\x08\x0a\x0c\x0e\x10\x12\x14\x16\x18\x1a\x1c\x1e])",
            "x",
            r"\d",
        ] {
            let fits = |n: u32| {
                units(&format!("(?:{atom}){{{n}}}")).is_some_and(|u| u <= MAX_REGEX_NFA_UNITS)
            };
            let (mut lo, mut hi) = (1, 2);
            while fits(hi) {
                lo = hi;
                hi *= 2;
            }
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if fits(mid) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let largest = format!("(?:{atom}){{{lo}}}");
            assert!(compile_mask(&largest).is_ok(), "{largest}");
            assert!(
                RegexBuilder::new(&largest)
                    .size_limit(REGEX_SIZE_LIMIT / 4)
                    .build()
                    .is_ok(),
                "{largest} needs more than a quarter of the byte limit"
            );
            assert_eq!(
                compile_mask(&format!("(?:{atom}){{{hi}}}")).unwrap_err(),
                MaskErrors {
                    invalid: false,
                    limit: true
                }
            );
        }
    }

    #[test]
    fn interner_is_dense_and_stable() {
        let mut i = Interner::new();
        assert_eq!(i.intern(b"a".to_vec()), 0);
        assert_eq!(i.intern(b"b".to_vec()), 1);
        assert_eq!(i.intern(b"a".to_vec()), 0);
        assert_eq!(i.num_tokens(), 2);
    }

    #[test]
    fn blank_lines() {
        assert!(is_blank(""));
        assert!(is_blank(" \t"));
        assert!(!is_blank(" x"));
    }

    use std::fmt::Write as _;

    use crate::line_diff::{self, LineAlgorithm};
    use crate::lines::split_lines;
    use crate::ops::{LineOp, full_ops};

    /// The reference path: every line interned with an owned key, the full
    /// sequences aligned and compacted by token.
    fn reference(
        builder: &KeyBuilder,
        algorithm: LineAlgorithm,
        left: &str,
        right: &str,
    ) -> (Vec<LineOp>, bool) {
        let (ll, rl) = (split_lines(left), split_lines(right));
        let mut interner = Interner::new();
        let (lt, rt) = intern_sides(builder, &mut interner, left, &ll, right, &rl);
        let mut alignment = line_diff::align(algorithm, &lt, &rt, interner.num_tokens());
        let approximate = line_diff::is_approximate(&lt, &rt, &alignment);
        let report = crate::slider::compact(
            &mut alignment,
            &mut crate::slider::TokenSide {
                tokens: &lt,
                content: |i: usize| ll[i].content(left).as_bytes(),
            },
            &mut crate::slider::TokenSide {
                tokens: &rt,
                content: |j: usize| rl[j].content(right).as_bytes(),
            },
            u64::MAX,
        );
        assert!(!report.exhausted);
        (full_ops(&alignment), approximate)
    }

    /// The large-input path: trimmed, borrowed or arena keys, core aligned,
    /// compacted over the whole files (runs may slide into the prefix and
    /// suffix, compared by key there).
    fn trimmed(
        builder: &KeyBuilder,
        algorithm: LineAlgorithm,
        left: &str,
        right: &str,
    ) -> (Vec<LineOp>, bool) {
        let (li, ri) = (LineIndex::new(left), LineIndex::new(right));
        let l = TextLines {
            text: left,
            lines: &li,
        };
        let r = TextLines {
            text: right,
            lines: &ri,
        };
        let core = tokenize_core(builder, l, r);
        let alignment = line_diff::align(algorithm, &core.left, &core.right, core.num_tokens);
        let approximate = is_approximate_core(builder, l, r, &core, &alignment);
        let (compacted, report) = compact_core(builder, l, r, &core, &alignment);
        assert!(!report.exhausted);
        (full_ops(&compacted), approximate)
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let x = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32;
            usize::try_from(x).unwrap() % bound
        }
    }

    /// Lines from a small vocabulary with whitespace, case and digit
    /// variants, so ignore options merge some of them.
    fn random_lines(rng: &mut Rng, n: usize) -> Vec<String> {
        const WORDS: [&str; 9] = ["a", "a ", " a", "A", "b\tc", "b  c", "id 12", "id 7", ""];
        (0..n)
            .map(|_| {
                if rng.next(4) == 0 {
                    format!("line {}", rng.next(40))
                } else {
                    WORDS[rng.next(WORDS.len())].to_owned()
                }
            })
            .collect()
    }

    fn join(rng: &mut Rng, lines: &[String], crlf_bias: usize) -> String {
        let mut text = String::new();
        for (i, line) in lines.iter().enumerate() {
            text.push_str(line);
            let last = i + 1 == lines.len();
            match rng.next(10) {
                _ if last && rng.next(3) == 0 => {}
                t if t < crlf_bias => text.push_str("\r\n"),
                9 => text.push('\r'),
                _ => text.push('\n'),
            }
        }
        text
    }

    fn edit(rng: &mut Rng, lines: &[String], edits: usize) -> Vec<String> {
        let mut out = lines.to_vec();
        for _ in 0..edits {
            let at = rng.next(out.len() + 1);
            match rng.next(3) {
                0 if at < out.len() => {
                    out.remove(at);
                }
                1 if at < out.len() => out[at] = random_lines(rng, 1).remove(0),
                _ => out.insert(at, random_lines(rng, 1).remove(0)),
            }
        }
        out
    }

    fn builders() -> Vec<KeyBuilder> {
        let mut options = vec![KeyOptions::default()];
        for whitespace in [
            WhitespaceMode::Trailing,
            WhitespaceMode::Change,
            WhitespaceMode::All,
        ] {
            options.push(KeyOptions {
                whitespace,
                ..KeyOptions::default()
            });
        }
        options.push(KeyOptions {
            ignore_case: true,
            ..KeyOptions::default()
        });
        options.push(KeyOptions {
            regex: vec![r"\d+".to_owned()],
            ..KeyOptions::default()
        });
        options
            .iter()
            .map(|o| KeyBuilder::new(o).unwrap())
            .collect()
    }

    const ALGORITHMS: [LineAlgorithm; 3] = [
        LineAlgorithm::Histogram,
        LineAlgorithm::Myers,
        LineAlgorithm::Patience,
    ];

    fn check(builder: &KeyBuilder, left: &str, right: &str, what: &str) {
        for algorithm in ALGORITHMS {
            assert_eq!(
                trimmed(builder, algorithm, left, right),
                reference(builder, algorithm, left, right),
                "{what} {algorithm:?} {left:?} {right:?}"
            );
        }
    }

    /// Trimming before interning, borrowed keys and the core-only
    /// `approximate` check give exactly the ops and flag of the reference.
    #[test]
    fn trimmed_tokenization_matches_the_reference_path() {
        let builders = builders();
        let mut rng = Rng(0x5EED_1B0B);
        for round in 0..400 {
            let n = rng.next(30);
            let base = random_lines(&mut rng, n);
            let left = join(&mut rng, &base, 1 + round % 4);
            let edits = rng.next(6);
            let edited = edit(&mut rng, &base, edits);
            let right = match round % 5 {
                0 => left.clone(),
                // Same lines, other terminators: equal under whitespace ignores.
                1 => join(&mut rng, &base, 5),
                _ => join(&mut rng, &edited, 1 + round % 4),
            };
            for (b, builder) in builders.iter().enumerate() {
                check(
                    builder,
                    &left,
                    &right,
                    &format!("round {round} builder {b}"),
                );
            }
        }
        // Edge texts around terminators at the trim boundaries.
        let edges = [
            "", "\n", "\r", "\r\n", "a", "a\n", "a\r", "a\r\n", "a\n\r", "\r\r\n", "x\ny",
        ];
        for left in edges {
            for right in edges {
                for builder in &builders {
                    check(builder, left, right, "edge");
                    let (l, r) = (format!("{left}m\n{left}"), format!("{right}m\n{right}"));
                    check(builder, &l, &r, "edge");
                }
            }
        }
        // The key prefix runs into the byte suffix (whitespace ignored).
        check(&builders[1], "a\na \na\n", "a\na\n", "overlap");
    }

    /// Changed lines whose keys occur only in the trimmed common lines still
    /// count towards `approximate`.
    #[test]
    fn approximate_counts_keys_in_the_trimmed_lines() {
        for (copies, want) in [(400, false), (600, true)] {
            let common: Vec<String> = (0..700).map(|i| format!("common {i}")).collect();
            let mut left = common.join("\n");
            let mut right = left.clone();
            for i in 0..copies {
                let _ = write!(left, "\nleft only {i}");
                let _ = write!(right, "\ncommon {i}");
            }
            left.push_str("\nend\n");
            right.push_str("\nend\n");
            for builder in &builders() {
                for algorithm in ALGORITHMS {
                    let got = trimmed(builder, algorithm, &left, &right);
                    assert_eq!(got.1, want, "{copies} copies");
                    assert_eq!(got, reference(builder, algorithm, &left, &right));
                }
            }
        }
    }

    /// The key as the straightforward definition builds it: mask, whitespace
    /// mode on a copy, then every character through the Unicode tables.
    fn reference_key(opts: &KeyOptions, content: &str, terminator: Terminator) -> Vec<u8> {
        let masks = compile_masks(&opts.regex).unwrap();
        let fold = CaseMapper::new();
        let piece = |out: &mut Vec<u8>, text: &str, at_end: bool| {
            let text = match opts.whitespace {
                WhitespaceMode::Trailing | WhitespaceMode::Change if at_end => {
                    text.trim_end_matches(is_whitespace)
                }
                _ => text,
            };
            let normalized: String = match opts.whitespace {
                WhitespaceMode::Change => collapse_runs(text).into_owned(),
                WhitespaceMode::All => text.chars().filter(|c| !is_whitespace(*c)).collect(),
                _ => text.to_owned(),
            };
            for c in normalized.chars() {
                let c = if opts.ignore_case {
                    fold.simple_fold(c)
                } else {
                    c
                };
                out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
            }
        };
        let mut out = Vec::new();
        let mut pos = 0;
        for (start, end, index) in masked_ranges(&masks, content) {
            piece(&mut out, &content[pos..start], false);
            out.extend_from_slice(&[MASK_BYTE, index]);
            pos = end;
        }
        piece(&mut out, &content[pos..], true);
        if opts.whitespace == WhitespaceMode::None {
            out.push(0xFF);
            out.push(match terminator {
                Terminator::Lf => b'n',
                Terminator::CrLf => b'w',
                Terminator::Cr => b'r',
                Terminator::None => b'0',
            });
        }
        out
    }

    /// The one-pass key builder (ASCII fast paths, no copies) gives the
    /// bytes of the reference definition for every option combination.
    #[test]
    fn one_pass_keys_match_the_reference() {
        let pieces = [
            "", " ", "\t", "\u{b}", "\u{c}", "  \t ", "a", "Z", "Hello", "WORLD", "ß", "ẞ", "İ",
            "K", "Ⱥ", "ſ", "ΣΑΣ", "Ǆ", "\u{a0}", "猫", "é", "e\u{301}", "🙂", "12:00:01", "x=1",
        ];
        let mut lines: Vec<String> = Vec::new();
        for a in pieces {
            for b in pieces {
                lines.push(format!("{a}{b}"));
                lines.push(format!(" {a} \t{b}  "));
                lines.push(format!("{a}  {b}\u{c}{a}"));
            }
        }
        let regexes = [Vec::new(), vec![r"\d+".to_owned(), "(?i)world".to_owned()]];
        let modes = [
            WhitespaceMode::None,
            WhitespaceMode::Trailing,
            WhitespaceMode::Change,
            WhitespaceMode::All,
        ];
        for whitespace in modes {
            for ignore_case in [false, true] {
                for regex in &regexes {
                    let opts = KeyOptions {
                        whitespace,
                        ignore_case,
                        regex: regex.clone(),
                    };
                    let builder = KeyBuilder::new(&opts).unwrap();
                    let mut buf = Vec::new();
                    for line in &lines {
                        builder.key_into(line, Terminator::CrLf, &mut buf);
                        let want = reference_key(&opts, line, Terminator::CrLf);
                        assert_eq!(buf, want, "{opts:?} {line:?}");
                    }
                }
            }
        }
    }

    /// The ASCII shortcut of [`simple_fold`] agrees with the Unicode tables.
    #[test]
    fn ascii_folding_matches_the_tables() {
        let tables = CaseMapper::new();
        for b in 0u8..=0x7F {
            let c = char::from(b);
            assert_eq!(simple_fold(c), tables.simple_fold(c), "{c:?}");
        }
    }

    /// A pure insertion or deletion is exact, even when the inserted lines
    /// repeat lines of the other side (no search runs on an empty core side).
    #[test]
    fn pure_insertions_are_never_approximate() {
        let base: String = (0..20).fold(String::new(), |mut s, i| {
            let _ = writeln!(s, "line {i}");
            s
        });
        let copies = "line 3\n".repeat(600);
        let middle = format!("{}{copies}{}", &base[..35], &base[35..]);
        for grown in [middle, format!("{copies}{base}"), format!("{base}{copies}")] {
            for builder in &builders() {
                for algorithm in ALGORITHMS {
                    let inserted = trimmed(builder, algorithm, &base, &grown);
                    assert!(!inserted.1, "{algorithm:?}");
                    assert_eq!(inserted, reference(builder, algorithm, &base, &grown));
                    let deleted = trimmed(builder, algorithm, &grown, &base);
                    assert!(!deleted.1, "{algorithm:?}");
                    assert_eq!(deleted, reference(builder, algorithm, &grown, &base));
                }
            }
        }
    }
}
