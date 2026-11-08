//! Diff Document types (`docs/spec/diff-document.md`, schema `0.2.0`).
//!
//! Writing: every field the spec defines is always serialized, `null` included
//! (spec §4).
//!
//! Reading (spec §2.2, §2.3):
//!
//! - Unknown fields are ignored.
//! - A missing known field reads as `null` where the field is nullable, `[]` or
//!   `{}` for arrays and objects, `Unknown` for enums, and the empty value of
//!   the type (`""`, `0`, `false`) otherwise.
//! - An unknown enum value maps to the `Unknown` variant. `Unknown` exists for
//!   reading only: serializing it is an error, so writers never emit it.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::options::Options;

/// The schema version written by this crate.
pub const SCHEMA_VERSION: &str = "0.2.0";

/// The Diff Document: diffyman's only result format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct DiffDocument {
    /// Semver string, `"0.2.0"` for this crate.
    pub schema_version: String,
    /// `"sha256:"` plus the hex SHA-256 of the canonical document without `id` and `provenance`.
    pub id: String,
    /// Document kind; `text` in 0.2.0.
    pub kind: Kind,
    /// Overall result.
    pub status: Status,
    /// Left input metadata.
    pub left: FileMeta,
    /// Right input metadata.
    pub right: FileMeta,
    /// Unit of inline offsets; equal to `options.positionEncoding`.
    pub position_encoding: PositionEncoding,
    /// Whole-file counts; `null` only when `status` is `error`.
    pub stats: Option<Stats>,
    /// Changed regions with context.
    pub hunks: Vec<Hunk>,
    /// Moved blocks (spec §6.5); `[]` unless `options.detectMoves` is true.
    pub moves: Vec<Move>,
    /// Structural changes (reserved; always `[]` in 0.2.0).
    pub structural: Vec<Value>,
    /// Table diff (reserved; always `null` in 0.2.0).
    pub table: Option<Value>,
    /// Page regions (reserved; always `[]` in 0.2.0).
    pub regions: Vec<Value>,
    /// Byte ranges (reserved; always `[]` in 0.2.0).
    pub bytes: Vec<Value>,
    /// Folder or archive tree (reserved; always `null` in 0.2.0).
    pub tree: Option<Value>,
    /// Warnings and errors, sorted by side then code.
    pub warnings: Vec<Warning>,
    /// The fully resolved options.
    pub options: Options,
    /// Audit data; excluded from `id` and from conformance.
    pub provenance: Provenance,
    /// Plugin data under `x-<vendor>` keys; `{}` in 0.2.0.
    pub extensions: BTreeMap<String, Value>,
}

/// The value a reader fills in for fields missing from a document (spec §2.2).
/// It is not a valid document to write.
impl Default for DiffDocument {
    fn default() -> DiffDocument {
        DiffDocument {
            schema_version: String::new(),
            id: String::new(),
            kind: Kind::Unknown,
            status: Status::Unknown,
            left: FileMeta::default(),
            right: FileMeta::default(),
            position_encoding: PositionEncoding::Unknown,
            stats: None,
            hunks: Vec::new(),
            moves: Vec::new(),
            structural: Vec::new(),
            table: None,
            regions: Vec::new(),
            bytes: Vec::new(),
            tree: None,
            warnings: Vec::new(),
            options: Options::unset(),
            provenance: Provenance::default(),
            extensions: BTreeMap::new(),
        }
    }
}

/// Document kind (open enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Plain text and code.
    Text,
    /// Reserved.
    Code,
    /// Reserved.
    Structured,
    /// Reserved.
    Table,
    /// Reserved.
    Document,
    /// Reserved.
    Image,
    /// Reserved.
    Binary,
    /// Reserved.
    Tree,
    /// A value this version does not know, or a missing field (read only).
    #[default]
    #[serde(other, skip_serializing)]
    #[schemars(skip)]
    Unknown,
}

/// Overall status (open enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// No change ops under the resolved options (`ignored` ops are not changes).
    Identical,
    /// At least one change op.
    Different,
    /// Reserved.
    Partial,
    /// At least one warning has severity `error`.
    Error,
    /// A value this version does not know, or a missing field (read only).
    #[default]
    #[serde(other, skip_serializing)]
    #[schemars(skip)]
    Unknown,
}

/// Unit of inline offsets (open enum).
///
/// `Default` is the option default `utf-8`; a missing document field reads as
/// `Unknown` instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub enum PositionEncoding {
    /// UTF-8 bytes (default).
    #[default]
    #[serde(rename = "utf-8")]
    Utf8,
    /// UTF-16 code units.
    #[serde(rename = "utf-16")]
    Utf16,
    /// Unicode scalar values.
    #[serde(rename = "codepoint")]
    Codepoint,
    /// A value this version does not know, or a missing field (read only).
    #[serde(other, skip_serializing)]
    #[schemars(skip)]
    Unknown,
}

/// Line terminator style of a file (open enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum LineTerminator {
    /// Every terminated line ends with LF.
    Lf,
    /// Every terminated line ends with CRLF.
    Crlf,
    /// Every terminated line ends with a lone CR.
    Cr,
    /// More than one style appears.
    Mixed,
    /// No line has a terminator.
    None,
    /// A value this version does not know, or a missing field (read only).
    #[default]
    #[serde(other, skip_serializing)]
    #[schemars(skip)]
    Unknown,
}

/// Metadata for one input (spec §6.1).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct FileMeta {
    /// Final path component given by the caller, or `null`.
    pub name: Option<String>,
    /// `"text/plain"` for kind `text`.
    pub media_type: Option<String>,
    /// Raw input length in bytes, BOM included.
    pub size: u64,
    /// Lowercase hex SHA-256 of the raw input.
    pub sha256: String,
    /// `"utf-8"` when the input decodes, else `null`.
    pub encoding: Option<String>,
    /// The raw input starts with EF BB BF.
    pub bom: bool,
    /// Terminator style, `null` if the input does not decode.
    pub line_terminator: Option<LineTerminator>,
    /// The last line has a terminator; `null` if the input does not decode.
    pub final_newline: Option<bool>,
    /// Number of lines; `null` if the input does not decode.
    pub line_count: Option<u64>,
}

/// Whole-file counts (spec §6.3).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Stats {
    /// Line count of the left side.
    pub lines_left: u64,
    /// Line count of the right side.
    pub lines_right: u64,
    /// Sum of right counts over `insert` and `replace` ops.
    pub lines_added: u64,
    /// Sum of left counts over `delete` and `replace` ops.
    pub lines_removed: u64,
    /// Lines covered by `equal` ops in the full op list.
    pub lines_unchanged: u64,
    /// Number of change ops.
    pub change_count: u64,
    /// Length of `hunks`.
    pub hunk_count: u64,
    /// 2U / (L + R), rounded half up to 4 decimals.
    pub similarity: Similarity,
}

/// A ratio in `[0, 1]` with 4-decimal precision, stored as basis points.
///
/// Serializes as the double nearest to `bp / 10000`, so JCS writes `0.75` and `1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Similarity(u16);

impl Similarity {
    /// Exactly 1.
    pub const ONE: Similarity = Similarity(10_000);

    /// Builds a value from basis points (1 bp = 0.0001). Values above 10000 are clamped.
    pub fn from_basis_points(bp: u32) -> Similarity {
        Similarity(u16::try_from(bp.min(10_000)).unwrap_or(10_000))
    }

    /// The value in basis points.
    pub fn basis_points(self) -> u32 {
        u32::from(self.0)
    }

    /// The value as a double (nearest to `bp / 10000`).
    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / 10_000.0
    }
}

impl Serialize for Similarity {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.0 == 10_000 {
            serializer.serialize_u64(1)
        } else if self.0 == 0 {
            serializer.serialize_u64(0)
        } else {
            serializer.serialize_f64(self.as_f64())
        }
    }
}

impl<'de> Deserialize<'de> for Similarity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(deserializer)?;
        if !(0.0..=1.0).contains(&value) {
            return Err(serde::de::Error::custom("similarity must be within [0, 1]"));
        }
        // In range by the check above, so the cast cannot truncate or wrap.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bp = (value * 10_000.0).round() as u32;
        Ok(Similarity::from_basis_points(bp))
    }
}

impl JsonSchema for Similarity {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Similarity".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "number",
            "minimum": 0,
            "maximum": 1,
            "description": "A ratio with 4-decimal precision."
        })
    }
}

/// A 1-based, half-open line range `{start, count}` (spec §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Range {
    /// First line (1-based); for an empty range, the line the insertion point precedes.
    pub start: u64,
    /// Number of lines.
    pub count: u64,
}

/// A group of ops with context (spec §6.4).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Hunk {
    /// `"h1"`, `"h2"`, ... in document order.
    pub id: String,
    /// Left lines covered by the ops.
    pub left: Range,
    /// Right lines covered by the ops.
    pub right: Range,
    /// Ops in tiling order.
    pub ops: Vec<Op>,
}

/// Op kind (open enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum OpKind {
    /// Lines that match.
    Equal,
    /// Lines only on the right.
    Insert,
    /// Lines only on the left.
    Delete,
    /// Lines on both sides that do not match.
    Replace,
    /// A change whose lines are all blank, ignored under `ignore.blankLines`
    /// (since 0.2.0). Not a change op; the counts may differ and either may be 0.
    Ignored,
    /// A value this version does not know, or a missing field (read only).
    #[default]
    #[serde(other, skip_serializing)]
    #[schemars(skip)]
    Unknown,
}

impl OpKind {
    /// True for `insert`, `delete` and `replace` (not for `equal` or `ignored`).
    pub fn is_change(self) -> bool {
        matches!(self, OpKind::Insert | OpKind::Delete | OpKind::Replace)
    }
}

/// One op of a hunk.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Op {
    /// Op kind.
    pub op: OpKind,
    /// Left lines.
    pub left_range: Range,
    /// Right lines.
    pub right_range: Range,
    /// Inline spans; `[]` unless granularity is not `line`, and only on `replace` ops.
    pub inline: Vec<Span>,
}

/// A side of the comparison (open enum).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    /// The left input.
    Left,
    /// The right input.
    Right,
    /// A value this version does not know, or a missing field (read only).
    #[default]
    #[serde(other, skip_serializing)]
    #[schemars(skip)]
    Unknown,
}

/// An inline change inside one line (spec §6.4).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Span {
    /// `left` (with `op: delete`) or `right` (with `op: insert`).
    pub side: Side,
    /// Absolute 1-based line number on that side.
    pub line: u64,
    /// Start offset from the start of the line content, in `positionEncoding` units.
    pub start: u64,
    /// End offset (exclusive).
    pub end: u64,
    /// `delete` or `insert`.
    pub op: OpKind,
}

/// Move kind (open enum, spec §6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MoveKind {
    /// The lines match pairwise.
    Pure,
    /// The lines match with small edits.
    Edited,
    /// A value this version does not know, or a missing field (read only).
    #[default]
    #[serde(other, skip_serializing)]
    #[schemars(skip)]
    Unknown,
}

/// One end of a moved block (spec §6.5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MoveEnd {
    /// `left` for `from`, `right` for `to`.
    pub side: Side,
    /// Lines of the block; `count` is at least 1.
    pub range: Range,
}

/// Reference to one op: hunk id and 0-based op index (spec §6.5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OpRef {
    /// Hunk id, such as `"h2"`.
    pub hunk: String,
    /// 0-based index of the op inside the hunk's `ops`.
    pub op: u64,
}

/// A moved block (spec §6.5). Emitted only when `options.detectMoves` is true.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Move {
    /// `"m1"`, `"m2"`, ... in the order of `moves[]`.
    pub id: String,
    /// `pure` or `edited`.
    pub kind: MoveKind,
    /// Source block, on the left.
    pub from: MoveEnd,
    /// Destination block, on the right.
    pub to: MoveEnd,
    /// Block similarity; exactly 1 for pure moves.
    pub similarity: Similarity,
    /// Every op the left range touches, in order.
    pub from_ops: Vec<OpRef>,
    /// Every op the right range touches, in order.
    pub to_ops: Vec<OpRef>,
}

/// Warning severity (open enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// The document is still valid.
    Warning,
    /// The document has `status: "error"`.
    Error,
    /// A value this version does not know, or a missing field (read only).
    #[default]
    #[serde(other, skip_serializing)]
    #[schemars(skip)]
    Unknown,
}

/// Stable warning and error codes (spec §10, the authoritative table).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum WarningCode {
    /// UTF-8 BOM removed before comparison.
    BomStripped,
    /// More than one terminator style.
    MixedLineEndings,
    /// Non-empty last line without a terminator.
    NoNewlineAtEof,
    /// The search was cut short.
    Approximate,
    /// Inline refinement skipped for `replace` ops over the size limits
    /// (granularity other than `line` only).
    RefineSkippedLimit,
    /// Moved-block detection skipped because too many lines changed
    /// (`detectMoves: true` only).
    MovesSkippedLimit,
    /// Input is not valid UTF-8.
    #[serde(rename = "invalid-utf8")]
    InvalidUtf8,
    /// An `ignore.regex` pattern does not compile (since 0.2.0).
    InvalidRegex,
    /// An option value is not supported by this engine version.
    UnsupportedOption,
    /// Input exceeds a size or complexity limit.
    LimitExceeded,
    /// A value this version does not know, or a missing field (read only).
    #[default]
    #[serde(other, skip_serializing)]
    #[schemars(skip)]
    Unknown,
}

impl WarningCode {
    /// Every code this version writes, in the order of the spec §10 table.
    pub const ALL: [WarningCode; 10] = [
        WarningCode::BomStripped,
        WarningCode::MixedLineEndings,
        WarningCode::NoNewlineAtEof,
        WarningCode::Approximate,
        WarningCode::RefineSkippedLimit,
        WarningCode::MovesSkippedLimit,
        WarningCode::InvalidUtf8,
        WarningCode::InvalidRegex,
        WarningCode::UnsupportedOption,
        WarningCode::LimitExceeded,
    ];

    /// The kebab-case code string (`"unknown"` for [`WarningCode::Unknown`],
    /// which is never written).
    pub fn as_str(self) -> &'static str {
        match self {
            WarningCode::BomStripped => "bom-stripped",
            WarningCode::MixedLineEndings => "mixed-line-endings",
            WarningCode::NoNewlineAtEof => "no-newline-at-eof",
            WarningCode::Approximate => "approximate",
            WarningCode::RefineSkippedLimit => "refine-skipped-limit",
            WarningCode::MovesSkippedLimit => "moves-skipped-limit",
            WarningCode::InvalidUtf8 => "invalid-utf8",
            WarningCode::InvalidRegex => "invalid-regex",
            WarningCode::UnsupportedOption => "unsupported-option",
            WarningCode::LimitExceeded => "limit-exceeded",
            WarningCode::Unknown => "unknown",
        }
    }

    /// The fixed English message for this code (spec §10).
    pub fn message(self) -> &'static str {
        match self {
            WarningCode::BomStripped => "UTF-8 byte order mark removed before comparison.",
            WarningCode::MixedLineEndings => "File uses more than one line terminator style.",
            WarningCode::NoNewlineAtEof => "No newline at end of file.",
            WarningCode::Approximate => {
                "Diff search was cut short; the result is valid but may not be minimal."
            }
            WarningCode::RefineSkippedLimit => {
                "Inline highlighting was skipped for changes that exceed size limits."
            }
            WarningCode::MovesSkippedLimit => {
                "Moved-block detection was skipped because too many lines changed."
            }
            WarningCode::InvalidUtf8 => "Input is not valid UTF-8.",
            WarningCode::InvalidRegex => {
                "An ignore.regex pattern is not a valid regular expression."
            }
            WarningCode::UnsupportedOption => {
                "An option value is not supported by this engine version."
            }
            WarningCode::LimitExceeded => "Input exceeds a configured size or complexity limit.",
            WarningCode::Unknown => "Unknown warning.",
        }
    }

    /// The severity that goes with this code.
    pub fn severity(self) -> Severity {
        match self {
            WarningCode::InvalidUtf8
            | WarningCode::InvalidRegex
            | WarningCode::UnsupportedOption
            | WarningCode::LimitExceeded => Severity::Error,
            WarningCode::Unknown => Severity::Unknown,
            _ => Severity::Warning,
        }
    }
}

/// A warning or error entry (spec §10).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Warning {
    /// Stable code; consumers act on this, never on the message.
    pub code: WarningCode,
    /// `warning` or `error`.
    pub severity: Severity,
    /// The side it applies to, or `null`.
    pub side: Option<Side>,
    /// Fixed English text for the code.
    pub message: String,
}

impl Warning {
    /// Builds the entry for `code` with its fixed severity and message.
    pub fn new(code: WarningCode, side: Option<Side>) -> Warning {
        Warning {
            code,
            severity: code.severity(),
            side,
            message: code.message().to_owned(),
        }
    }

    /// Sort key of spec §10: side (`left`, `right`, `null`), then code in code-point order.
    pub fn sort_key(&self) -> (u8, &'static str) {
        let side = match self.side {
            Some(Side::Left) => 0,
            Some(Side::Right) => 1,
            Some(Side::Unknown) => 2,
            None => 3,
        };
        (side, self.code.as_str())
    }
}

/// Audit data (spec §11). Excluded from `id` and from conformance.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Provenance {
    /// Engine name and version.
    pub engine: NameVersion,
    /// Algorithm ID, such as `text.histogram@1`.
    pub algorithm: String,
    /// Adapter id and version.
    pub adapter: AdapterInfo,
    /// Surface that produced the document: `rust`, `cli`, `python`, `node`, `wasm`, `java` or `c`.
    pub binding: String,
    /// Operating system and CPU architecture.
    pub platform: Platform,
    /// Timing; the only place timing values appear.
    pub timing: Timing,
    /// Further keys, such as library versions.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A name and a version.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct NameVersion {
    /// Name.
    pub name: String,
    /// Version.
    pub version: String,
}

/// Adapter identity.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AdapterInfo {
    /// Adapter id, such as `text`.
    pub id: String,
    /// Adapter version.
    pub version: String,
}

/// Platform identity.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Platform {
    /// Operating system, as Rust's `std::env::consts::OS`, or `unknown` where
    /// that is empty (`wasm32-unknown-unknown`).
    pub os: String,
    /// CPU architecture, as Rust's `std::env::consts::ARCH`.
    pub arch: String,
}

/// Timing values.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Timing {
    /// Wall-clock time of the whole diff in microseconds.
    pub total_micros: u64,
}
