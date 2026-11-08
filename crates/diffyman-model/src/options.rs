//! Diff options (spec §9): the resolved form echoed in every document and the
//! input form in which every key is optional.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::document::PositionEncoding;

/// Largest allowed `context` value (2^31 − 1).
pub const MAX_CONTEXT: u32 = 2_147_483_647;

/// Line diff algorithm (open enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Algorithm {
    /// `text.histogram@1` (default).
    #[default]
    Histogram,
    /// `text.myers@1`.
    Myers,
    /// `text.patience@1`.
    Patience,
    /// A value this version does not know.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

impl Algorithm {
    /// The versioned algorithm ID recorded in provenance (spec §12).
    pub fn id(self) -> &'static str {
        match self {
            Algorithm::Histogram => "text.histogram@1",
            Algorithm::Myers => "text.myers@1",
            Algorithm::Patience => "text.patience@1",
            Algorithm::Unknown => "text.unknown",
        }
    }
}

/// Inline granularity (open enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Granularity {
    /// No inline spans (default).
    #[default]
    Line,
    /// UAX #29 word segments (provisional).
    Word,
    /// Extended grapheme clusters (provisional).
    Char,
    /// Chosen per op (provisional).
    Smart,
    /// A value this version does not know.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

/// Whitespace handling for line comparison (open enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum WhitespaceMode {
    /// Content plus terminator kind (default).
    #[default]
    None,
    /// Ignore trailing whitespace.
    Trailing,
    /// Ignore changes in the amount of whitespace (`git diff -b`).
    Change,
    /// Ignore all whitespace.
    All,
    /// A value this version does not know.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

/// Resolved ignore rules.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IgnoreOptions {
    /// Whitespace mode.
    pub whitespace: WhitespaceMode,
    /// Unicode simple case folding.
    pub case: bool,
    /// Ignore blank lines (deferred to 0.2.0).
    pub blank_lines: bool,
    /// Regex masks (deferred to 0.2.0).
    pub regex: Vec<String>,
}

/// Fully resolved options, echoed in every document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    /// Line diff algorithm.
    pub algorithm: Algorithm,
    /// Inline granularity.
    pub granularity: Granularity,
    /// Ignore rules.
    pub ignore: IgnoreOptions,
    /// Context lines around changes.
    pub context: u32,
    /// Unit of inline offsets.
    pub position_encoding: PositionEncoding,
    /// Moved-block detection (deferred).
    pub detect_moves: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            algorithm: Algorithm::Histogram,
            granularity: Granularity::Line,
            ignore: IgnoreOptions::default(),
            context: 3,
            position_encoding: PositionEncoding::Utf8,
            detect_moves: false,
        }
    }
}

// The input types below use `Option` only to tell "missing" from "given".
// `null` is not a valid value, so the schema describes the inner types.

/// Ignore rules as given by a caller; every key is optional.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IgnoreOptionsInput {
    /// Whitespace mode (default `none`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "WhitespaceMode")]
    pub whitespace: Option<WhitespaceMode>,
    /// Case folding (default `false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool")]
    pub case: Option<bool>,
    /// Ignore blank lines (default `false`; `true` is deferred to 0.2.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool")]
    pub blank_lines: Option<bool>,
    /// Regex masks (default `[]`; non-empty is deferred to 0.2.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<String>")]
    pub regex: Option<Vec<String>>,
}

/// Options as given by a caller (`options.json`); every key is optional.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptionsInput {
    /// Algorithm (default `histogram`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Algorithm")]
    pub algorithm: Option<Algorithm>,
    /// Granularity (default `line`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Granularity")]
    pub granularity: Option<Granularity>,
    /// Ignore rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "IgnoreOptionsInput")]
    pub ignore: Option<IgnoreOptionsInput>,
    /// Context lines, 0 to 2^31 − 1 (default 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u32", range(max = 2_147_483_647))]
    pub context: Option<u64>,
    /// Position encoding (default `utf-8`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "PositionEncoding")]
    pub position_encoding: Option<PositionEncoding>,
    /// Moved-block detection (default `false`; `true` is deferred).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool")]
    pub detect_moves: Option<bool>,
}
