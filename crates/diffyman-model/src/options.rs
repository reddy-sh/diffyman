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
    /// A value this version does not know, or a missing field (read only).
    #[serde(other, skip_serializing)]
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
    /// UAX #29 word segments.
    Word,
    /// Extended grapheme clusters.
    Char,
    /// Chosen per op.
    Smart,
    /// A value this version does not know, or a missing field (read only).
    #[serde(other, skip_serializing)]
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
    /// A value this version does not know, or a missing field (read only).
    #[serde(other, skip_serializing)]
    #[schemars(skip)]
    Unknown,
}

/// Which adapter compares the inputs (open enum, since 0.3.0; spec §16.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Adapter {
    /// Text when both inputs are text, otherwise binary (default).
    #[default]
    Auto,
    /// Always the text adapter: an input that is not UTF-8 is an `invalid-utf8` error.
    Text,
    /// Always the binary adapter, even for text.
    Binary,
    /// Always the tree adapter: an input that is not an archive is an
    /// `invalid-archive` error (spec §19.1).
    Tree,
    /// A value this version does not know, or a missing field (read only).
    #[serde(other, skip_serializing)]
    #[schemars(skip)]
    Unknown,
}

/// Largest allowed `tree.maxDepth`.
pub const MAX_TREE_DEPTH: u32 = 16;

/// Largest allowed `tree.childBudget` (2^31 − 1).
pub const MAX_CHILD_BUDGET: u32 = 2_147_483_647;

/// Resolved options of the tree adapter (since 0.3.0, spec §9, §19).
/// `Default` gives the spec defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default = "TreeOptions::unset")]
pub struct TreeOptions {
    /// Folders only: list what a symbolic link points to instead of the link.
    pub follow_symlinks: bool,
    /// Folders only: honor `.gitignore` and `.ignore` files inside the folders.
    pub ignore_files: bool,
    /// `.gitignore`-style patterns matched against entry paths on both sides.
    pub exclude: Vec<String>,
    /// Pair left-only and right-only files with equal content as `moved`.
    pub detect_moves: bool,
    /// How many levels of nested archives are read as trees.
    pub max_depth: u32,
    /// Bytes of canonical JSON the child documents may take in total.
    pub child_budget: u32,
}

impl Default for TreeOptions {
    fn default() -> TreeOptions {
        TreeOptions {
            follow_symlinks: false,
            ignore_files: false,
            exclude: Vec::new(),
            detect_moves: true,
            max_depth: 3,
            child_budget: 16 << 20,
        }
    }
}

impl TreeOptions {
    /// What a reader fills in for keys missing from a document's
    /// `options.tree` (see [`Options::unset`]).
    pub fn unset() -> TreeOptions {
        TreeOptions {
            follow_symlinks: false,
            ignore_files: false,
            exclude: Vec::new(),
            detect_moves: false,
            max_depth: 0,
            child_budget: 0,
        }
    }
}

/// Resolved ignore rules. `Default` gives the spec defaults.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default = "IgnoreOptions::unset")]
pub struct IgnoreOptions {
    /// Whitespace mode.
    pub whitespace: WhitespaceMode,
    /// Unicode simple case folding.
    pub case: bool,
    /// Ignore changes whose lines are all blank: they become `ignored` ops.
    pub blank_lines: bool,
    /// Patterns whose matches are masked before comparison, in priority order.
    pub regex: Vec<String>,
}

/// Fully resolved options, echoed in every document. `Default` gives the spec
/// defaults.
///
/// When read from a document, a missing key takes the value of
/// [`Options::unset`] (spec §2.2), not the option default: the document does
/// not say which value was used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default = "Options::unset")]
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
    /// Moved-block detection (`text.moves@1`, pure and edited moves).
    pub detect_moves: bool,
    /// Adapter selection (since 0.3.0).
    pub adapter: Adapter,
    /// Folder and archive options (since 0.3.0).
    pub tree: TreeOptions,
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
            adapter: Adapter::Auto,
            tree: TreeOptions::default(),
        }
    }
}

impl Options {
    /// What a reader fills in for keys missing from a document's `options`:
    /// every enum `Unknown`, `context` 0, booleans `false`, `regex` empty.
    /// Not a valid input to the engine.
    pub fn unset() -> Options {
        Options {
            algorithm: Algorithm::Unknown,
            granularity: Granularity::Unknown,
            ignore: IgnoreOptions::unset(),
            context: 0,
            position_encoding: PositionEncoding::Unknown,
            detect_moves: false,
            adapter: Adapter::Unknown,
            tree: TreeOptions::unset(),
        }
    }
}

impl IgnoreOptions {
    /// What a reader fills in for keys missing from a document's
    /// `options.ignore` (see [`Options::unset`]).
    pub fn unset() -> IgnoreOptions {
        IgnoreOptions {
            whitespace: WhitespaceMode::Unknown,
            case: false,
            blank_lines: false,
            regex: Vec::new(),
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
    /// Ignore changes whose lines are all blank (default `false`). They become
    /// `ignored` ops, which are not changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool")]
    pub blank_lines: Option<bool>,
    /// Regular expressions (Rust `regex` syntax) whose matches are masked
    /// before lines are compared (default `[]`). At most 16 patterns of at
    /// most 1024 characters each; a pattern that does not compile gives an
    /// `invalid-regex` error, one over a limit gives `limit-exceeded`.
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
    /// Moved-block detection (default `false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool")]
    pub detect_moves: Option<bool>,
    /// Adapter selection (default `auto`): `auto`, `text`, `binary` or `tree`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Adapter")]
    pub adapter: Option<Adapter>,
    /// Folder and archive options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "TreeOptionsInput")]
    pub tree: Option<TreeOptionsInput>,
}

/// Tree options as given by a caller; every key is optional.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TreeOptionsInput {
    /// Folders only: follow symbolic links (default `false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool")]
    pub follow_symlinks: Option<bool>,
    /// Folders only: honor `.gitignore` and `.ignore` files (default `false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool")]
    pub ignore_files: Option<bool>,
    /// `.gitignore`-style patterns to leave out (default `[]`). At most 64
    /// patterns of at most 1024 characters; one that does not compile gives
    /// an `invalid-glob` error, too many or too long gives `limit-exceeded`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<String>")]
    pub exclude: Option<Vec<String>>,
    /// Pair files with equal content as `moved` (default `true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "bool")]
    pub detect_moves: Option<bool>,
    /// Nested archive levels read as trees, 0 to 16 (default 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u32", range(max = 16))]
    pub max_depth: Option<u64>,
    /// Total bytes of embedded child documents, 0 to 2^31 − 1 (default 16777216).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u32", range(max = 2_147_483_647))]
    pub child_budget: Option<u64>,
}
