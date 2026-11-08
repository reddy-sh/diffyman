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

use crate::lines::{Line, Terminator};

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

/// Key byte that starts a placeholder. It never occurs in UTF-8.
const MASK_BYTE: u8 = 0xFE;

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
}

/// Applies Unicode simple case folding (CaseFolding.txt statuses C and S).
pub fn simple_fold(c: char) -> char {
    CaseMapper::new().simple_fold(c)
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
        })
    }

    /// The masked parts of `content` as `(start, end, pattern)` byte ranges,
    /// sorted and disjoint (spec §7.2 step 1).
    ///
    /// Each pattern's non-empty matches are found over the whole content,
    /// leftmost-first and without overlaps. Patterns are applied in order: a
    /// match is kept unless it overlaps a match kept for an earlier pattern.
    pub fn masked_ranges(&self, content: &str) -> Vec<(usize, usize, u8)> {
        let mut kept: Vec<(usize, usize, u8)> = Vec::new();
        for (index, mask) in self.masks.iter().enumerate() {
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

    /// The key of one line, as bytes. Distinct terminators give distinct keys
    /// only under whitespace mode `none`.
    pub fn key(&self, content: &str, terminator: Terminator) -> Vec<u8> {
        let mut out = Vec::with_capacity(content.len() + 2);
        let mut pos = 0;
        if !self.masks.is_empty() {
            for (start, end, index) in self.masked_ranges(content) {
                self.push_text(&mut out, &content[pos..start], false);
                out.push(MASK_BYTE);
                out.push(index);
                pos = end;
            }
        }
        self.push_text(&mut out, &content[pos..], true);
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
        out
    }

    /// Appends one unmasked piece of a line. A placeholder is not whitespace:
    /// runs never cross it, and only the piece at the end of the line loses
    /// its trailing whitespace.
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
        let normalized = match self.whitespace {
            WhitespaceMode::None | WhitespaceMode::Trailing => std::borrow::Cow::Borrowed(text),
            WhitespaceMode::Change => collapse_runs(text),
            WhitespaceMode::All => {
                std::borrow::Cow::Owned(text.chars().filter(|c| !is_whitespace(*c)).collect())
            }
        };
        match self.fold {
            Some(fold) => {
                let mut buf = [0; 4];
                for c in normalized.chars() {
                    out.extend_from_slice(fold.simple_fold(c).encode_utf8(&mut buf).as_bytes());
                }
            }
            None => out.extend_from_slice(normalized.as_bytes()),
        }
    }
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
}
