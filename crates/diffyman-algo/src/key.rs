//! Line comparison keys and interning (Diff Document spec §7.2).
//!
//! Two lines are equal when their keys are equal. The key is built from the
//! line content in this order: regex masks, whitespace mode, case folding, and
//! (for whitespace mode `none` only) the terminator kind.

use std::collections::HashMap;

use icu_casemap::{CaseMapper, CaseMapperBorrowed};
use regex::Regex;

use crate::lines::{Line, Terminator};

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
    /// Regex patterns whose matches are removed from the key, in order.
    pub regex: Vec<String>,
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

impl KeyBuilder {
    /// Compiles the options. Fails only on an invalid regex.
    pub fn new(options: &KeyOptions) -> Result<KeyBuilder, regex::Error> {
        let masks = options
            .regex
            .iter()
            .map(|p| Regex::new(p))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(KeyBuilder {
            whitespace: options.whitespace,
            fold: options.ignore_case.then(CaseMapper::new),
            masks,
        })
    }

    /// The key of one line, as bytes. Distinct terminators give distinct keys
    /// only under whitespace mode `none`.
    pub fn key(&self, content: &str, terminator: Terminator) -> Vec<u8> {
        let mut masked = std::borrow::Cow::Borrowed(content);
        for mask in &self.masks {
            let replaced = mask.replace_all(&masked, "").into_owned();
            masked = std::borrow::Cow::Owned(replaced);
        }
        let normalized = normalize_whitespace(&masked, self.whitespace);
        let mut out = String::with_capacity(normalized.len());
        match self.fold {
            Some(fold) => out.extend(normalized.chars().map(|c| fold.simple_fold(c))),
            None => out.push_str(&normalized),
        }
        let mut bytes = out.into_bytes();
        if self.whitespace == WhitespaceMode::None {
            // 0xFF never occurs in UTF-8, so the suffix cannot collide with content.
            bytes.push(0xFF);
            bytes.push(match terminator {
                Terminator::Lf => b'n',
                Terminator::CrLf => b'w',
                Terminator::Cr => b'r',
                Terminator::None => b'0',
            });
        }
        bytes
    }
}

/// Applies a whitespace mode to text (no terminator handling).
pub fn normalize_whitespace(content: &str, mode: WhitespaceMode) -> std::borrow::Cow<'_, str> {
    match mode {
        WhitespaceMode::None => std::borrow::Cow::Borrowed(content),
        WhitespaceMode::Trailing => {
            std::borrow::Cow::Borrowed(content.trim_end_matches(is_whitespace))
        }
        WhitespaceMode::Change => {
            let trimmed = content.trim_end_matches(is_whitespace);
            let mut out = String::with_capacity(trimmed.len());
            let mut in_run = false;
            for c in trimmed.chars() {
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

    #[test]
    fn regex_masks() {
        let o = KeyOptions {
            regex: vec![r"\d+".to_owned(), "x".to_owned()],
            ..KeyOptions::default()
        };
        assert_eq!(
            key(&o, "id 123x", Terminator::Lf),
            key(&o, "id 9", Terminator::Lf)
        );
        assert!(
            KeyBuilder::new(&KeyOptions {
                regex: vec!["(".to_owned()],
                ..KeyOptions::default()
            })
            .is_err()
        );
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
