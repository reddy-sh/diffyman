//! diffyman: compare two inputs and get one Diff Document.
//!
//! The cross-language contract is the **canonical JSON** (RFC 8785) returned by
//! [`diff`]: every binding returns the same bytes once the top-level
//! `provenance` key is removed. The typed API ([`diff_document`]) returns the
//! same document as Rust values.
//!
//! ```
//! let json = diffyman::diff(b"a\nb\n", b"a\nc\n", r#"{"context": 1}"#).unwrap();
//! assert!(json.contains(r#""status":"different""#));
//! ```
//!
//! Behavior follows `docs/spec/diff-document.md` (schema `0.4.0`):
//!
//! - Each input is detected as text or binary, with a media type
//!   (`diffyman-detect`, spec §16). With `adapter: "auto"` (the default), two
//!   UTF-8 texts give `kind: "text"`; anything else gives `kind: "binary"`,
//!   compared byte by byte (`diffyman-adapter-binary`, spec §17), with a
//!   `binary-fallback` warning when one side is text.
//! - A UTF-8 BOM is removed before comparison (`bom-stripped`).
//! - Lines end with LF, CRLF or a lone CR. A missing final newline is reported
//!   (`no-newline-at-eof`) and, under whitespace mode `none`, compares as a change.
//! - With `adapter: "text"`, an input that is not UTF-8 gives `status: "error"`
//!   with an `invalid-utf8` error.
//! - `ignore.regex` masks matches before comparison; `ignore.blankLines` turns
//!   blank-only changes into `ignored` ops, which are not changes.
//! - Two archives (zip, tar, gzip- or zstd-compressed tar) give `kind: "tree"`
//!   (`diffyman-adapter-tree`, spec §19): one entry per path, renames found by
//!   content, and a child document for each changed file. With the `fs`
//!   feature (on by default), `diff_paths` also compares folders on disk.
//!   Archives are never extracted, and their paths are never resolved.
//! - Invalid options are rejected with [`Error::InvalidOptions`]. A regex or
//!   `tree.exclude` pattern that does not compile, or is over a limit, gives a
//!   document with an `invalid-regex`, `invalid-glob` or `limit-exceeded` error.
//! - Timing appears only in `provenance`.
//!
//! Features: `archives` (archives as trees; on with `fs`, which is on by
//! default) and `fs` (folders on disk). A build without `archives` gives the
//! same document as any other build for every pair it compares, and refuses
//! the pairs that need archive support with [`Error::Unsupported`] (spec
//! §19.7): adapter `tree`, a non-empty `tree.exclude`, or, with adapter
//! `auto`, two inputs that both start like an archive.

mod context;
mod options;
#[cfg(feature = "fs")]
mod read;
#[cfg(feature = "archives")]
mod tree;

use std::collections::BTreeMap;

use diffyman_adapter_binary as binary;
pub use diffyman_algo::lines::{
    IndexedText, Line, LineCursor, LineIndex, Terminator, TextInputs, split_lines,
};
use diffyman_detect::{self as detect, Class, Detection, Encoding};
pub use diffyman_model as model;
use diffyman_model::{
    Adapter, AdapterInfo, ByteRange, ByteStats, DiffDocument, FileMeta, Granularity, Hunk,
    JcsError, Kind, LineTerminator, Move, MoveEnd, MoveKind, NameVersion, Op, OpKind, OpRef,
    Options, Platform, PositionEncoding, Provenance, Range, SCHEMA_VERSION, Side, Similarity, Span,
    Stats, Status, Timing, Warning, WarningCode, WhitespaceMode,
};
pub use options::{
    parse_options, parse_options_json, parse_options_value, resolve_options, validate_options,
};
#[cfg(feature = "fs")]
pub use read::{MAX_STREAM_BYTES, read_path};
use serde_json::Value;
#[cfg(feature = "fs")]
pub use tree::{diff_paths, diff_paths_with_limit};

use context::Context;
use diffyman_algo::key::{self, KeyBuilder, KeyOptions, TextLines};
use diffyman_algo::line_diff::{self, LineAlgorithm};
use diffyman_algo::moves;
use diffyman_algo::ops::{self as algo_ops, LineOp};
use diffyman_algo::refine::{self, RefineOptions, Refined, SideLine, SpanSide};

/// The pinned archive libraries, in provenance when the build has them.
#[cfg(feature = "archives")]
const TREE_LIBRARIES: &[(&str, &str)] = &tree::LIBRARY_VERSIONS;
#[cfg(not(feature = "archives"))]
const TREE_LIBRARIES: &[(&str, &str)] = &[];

/// Engine name recorded in provenance.
pub const ENGINE_NAME: &str = "diffyman";
/// Engine version recorded in provenance.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Errors that prevent a document from being produced.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The options object is invalid (unknown key, wrong type or unknown value).
    #[error("invalid options: {0}")]
    InvalidOptions(String),
    /// The document could not be serialized canonically.
    #[error(transparent)]
    Canonical(#[from] JcsError),
    /// An input path could not be read (`diff_paths` only, feature `fs`).
    #[error("{0}")]
    Io(String),
    /// This build cannot compare these inputs with these options: it was
    /// built without a feature they need (spec §19.7). No document is
    /// produced; a build with the feature gives the document.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl Error {
    /// A stable code for the kind of error, for bindings that report errors
    /// as values: `invalid-options`, `canonical`, `io` or `unsupported`.
    pub fn code(&self) -> &'static str {
        match self {
            Error::InvalidOptions(_) => "invalid-options",
            Error::Canonical(_) => "canonical",
            Error::Io(_) => "io",
            Error::Unsupported(_) => "unsupported",
        }
    }
}

/// The features of this build that change which inputs it compares:
/// `archives` (archives as trees) and `fs` (folders on disk).
pub const FEATURES: &[&str] = &[
    #[cfg(feature = "archives")]
    "archives",
    #[cfg(feature = "fs")]
    "fs",
];

/// True when `bytes` start like an archive (spec §19.7): a zip signature, a
/// tar header, or the magic number of a gzip stream, a zstd frame or a zstd
/// skippable frame. Every input the tree adapter reads as an archive starts
/// like this, so a build without archive support can refuse exactly what it
/// cannot compare without decompressing anything; it also refuses gzip and
/// zstd data that holds no tar.
pub fn starts_like_archive(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || (bytes.len() >= 512 && &bytes[257..262] == b"ustar")
        || bytes.starts_with(&[0x1f, 0x8b])
        || bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd])
        || (bytes.len() >= 4 && bytes[0] & 0xf0 == 0x50 && bytes[1..4] == [0x2a, 0x4d, 0x18])
}

/// Without the `archives` feature: [`Error::Unsupported`] for the inputs and
/// options a full build would read as archives (spec §19.7).
#[cfg(not(feature = "archives"))]
fn check_supported(left: &[u8], right: &[u8], options: &Options) -> Result<(), Error> {
    let reason = if options.adapter == Adapter::Tree {
        "adapter \"tree\" needs archive support"
    } else if !options.tree.exclude.is_empty() {
        "tree.exclude needs archive support"
    } else if options.adapter == Adapter::Auto
        && starts_like_archive(left)
        && starts_like_archive(right)
    {
        "both inputs start like archives, and this build has no archive support"
    } else {
        return Ok(());
    };
    Err(Error::Unsupported(format!(
        "{reason} (built without the `archives` feature)"
    )))
}

/// With the `archives` feature every input is supported.
#[cfg(feature = "archives")]
#[allow(clippy::unnecessary_wraps)]
fn check_supported(_left: &[u8], _right: &[u8], _options: &Options) -> Result<(), Error> {
    Ok(())
}

/// One input: its bytes and an optional name.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    bytes: &'a [u8],
    name: Option<&'a str>,
}

impl<'a> Input<'a> {
    /// An input with no name (`name: null` in the document).
    pub fn new(bytes: &'a [u8]) -> Input<'a> {
        Input { bytes, name: None }
    }

    /// An input with a name. Only the final path component is kept.
    pub fn named(bytes: &'a [u8], name: &'a str) -> Input<'a> {
        let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
        Input {
            bytes,
            name: (!last.is_empty()).then_some(last),
        }
    }

    /// The raw bytes.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The name as it will appear in the document.
    pub fn name(&self) -> Option<&'a str> {
        self.name
    }
}

/// Settings that affect only `provenance`.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The surface producing the document: `rust`, `cli`, `python`, `node`, `wasm`, `java` or `c`.
    pub binding: String,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            binding: "rust".to_owned(),
        }
    }
}

/// Diffs two unnamed inputs and returns the canonical JSON Diff Document.
///
/// `options_json` is an options object (spec §9); blank text means `{}`.
pub fn diff(left: &[u8], right: &[u8], options_json: &str) -> Result<String, Error> {
    diff_inputs(Input::new(left), Input::new(right), options_json)
}

/// Diffs two inputs and returns the canonical JSON Diff Document.
pub fn diff_inputs(left: Input<'_>, right: Input<'_>, options_json: &str) -> Result<String, Error> {
    diff_inputs_with(left, right, options_json, &Settings::default())
}

/// Like [`diff_inputs`], with provenance settings. This is the JSON-in/JSON-out
/// entry point every binding calls.
pub fn diff_inputs_with(
    left: Input<'_>,
    right: Input<'_>,
    options_json: &str,
    settings: &Settings,
) -> Result<String, Error> {
    let options = parse_options(options_json)?;
    let doc = diff_document_with(left, right, &options, settings)?;
    Ok(diffyman_model::to_canonical_json(&doc)?)
}

/// Diffs two inputs with resolved options and returns the typed document.
///
/// `options` is checked with [`validate_options`], the same rules as the JSON
/// path: an `Unknown` enum value or an out-of-range `context` is
/// [`Error::InvalidOptions`] and no document is produced.
pub fn diff_document(
    left: Input<'_>,
    right: Input<'_>,
    options: &Options,
) -> Result<DiffDocument, Error> {
    diff_document_with(left, right, options, &Settings::default())
}

/// Like [`diff_document`], with provenance settings.
pub fn diff_document_with(
    left: Input<'_>,
    right: Input<'_>,
    options: &Options,
    settings: &Settings,
) -> Result<DiffDocument, Error> {
    validate_options(options)?;
    check_supported(left.bytes, right.bytes, options)?;
    let timer = Timer::start();
    let mut ctx = Context::new(options);
    let doc = document(left, right, options, &mut ctx);
    finish_document(doc, options, settings, &timer)
}

/// Like [`diff_document_with`], and for a text document whose inputs both
/// decode as UTF-8, also those inputs as the comparison read them (without
/// their BOM, split into lines), so a renderer can show their lines without
/// hashing, decoding and splitting the inputs again. `None` for other
/// documents. The document is the one [`diff_document_with`] returns.
pub fn diff_document_with_text<'a>(
    left: Input<'a>,
    right: Input<'a>,
    options: &Options,
    settings: &Settings,
) -> Result<(DiffDocument, Option<TextInputs<'a>>), Error> {
    validate_options(options)?;
    check_supported(left.bytes, right.bytes, options)?;
    let timer = Timer::start();
    let mut ctx = Context::new(options);
    let (doc, text) = document_with_text(left, right, options, &mut ctx);
    Ok((finish_document(doc, options, settings, &timer)?, text))
}

/// Fills in provenance and `id`.
fn finish_document(
    mut doc: DiffDocument,
    options: &Options,
    settings: &Settings,
    timer: &Timer,
) -> Result<DiffDocument, Error> {
    doc.provenance = provenance(doc.kind, options, settings, 0);
    doc.id = diffyman_model::compute_id(&doc)?;
    doc.provenance.timing.total_micros = timer.elapsed_micros();
    Ok(doc)
}

/// The document of two inputs, without `id` and provenance. Child documents
/// of a tree come here too, one level deeper in `ctx` (spec §19.5).
fn document(
    left: Input<'_>,
    right: Input<'_>,
    options: &Options,
    ctx: &mut Context,
) -> DiffDocument {
    document_with_text(left, right, options, ctx).0
}

/// [`document`], and both decoded inputs when it is a text document whose
/// inputs both decode.
fn document_with_text<'a>(
    left: Input<'a>,
    right: Input<'a>,
    options: &Options,
    ctx: &mut Context,
) -> (DiffDocument, Option<TextInputs<'a>>) {
    #[cfg(feature = "archives")]
    let depth_limited = {
        let archives = tree::archive_format(left.bytes).is_some()
            && tree::archive_format(right.bytes).is_some();
        let below_depth = ctx.depth > options.tree.max_depth;
        match options.adapter {
            Adapter::Tree => return (tree::archive_document(left, right, options, ctx), None),
            Adapter::Auto if archives && !below_depth => {
                return (tree::archive_document(left, right, options, ctx), None);
            }
            _ => {}
        }
        options.adapter == Adapter::Auto && archives
    };
    // Without archive support, the pairs that would be trees were refused
    // before this (`check_supported`), and there are no child documents.
    #[cfg(not(feature = "archives"))]
    let depth_limited = false;
    // Byte-identical inputs are detected and decoded once: same hash, same
    // lines (shared), same per-side warnings. Only the name (and so the media
    // type from the extension) can differ.
    let (l, r) = if left.bytes == right.bytes {
        let l = decode_side(left, options.adapter);
        let detection = l.detection.renamed(right.bytes, right.name);
        let r = DecodedSide {
            meta: FileMeta {
                name: right.name.map(str::to_owned),
                media_type: Some(detection.media_type.to_owned()),
                ..l.meta.clone()
            },
            detection,
            text: l.text.clone(),
            codes: l.codes.clone(),
        };
        (l, r)
    } else {
        decode_pair(left, right, options.adapter)
    };
    let kind = match options.adapter {
        Adapter::Text => Kind::Text,
        Adapter::Binary => Kind::Binary,
        _ if depth_limited => Kind::Binary,
        _ if l.is_utf8_text() && r.is_utf8_text() => Kind::Text,
        _ => Kind::Binary,
    };

    // Option errors apply whatever the kind (spec §7.2.1, §9).
    let mut warnings = ctx.option_warnings.clone();
    if depth_limited {
        warnings.push(Warning::new(WarningCode::DepthLimit, None));
    }
    if kind == Kind::Binary {
        let doc = binary_document(left.bytes, right.bytes, &l, &r, options, warnings);
        (doc, None)
    } else {
        text_document(l, r, ctx.key_builder.as_ref(), options, warnings)
    }
}

/// The comparison key builder of the options, or the option errors that stop
/// it (spec §7.2.1).
fn key_builder(options: &Options, warnings: &mut Vec<Warning>) -> Option<KeyBuilder> {
    match KeyBuilder::new(&KeyOptions {
        whitespace: whitespace_mode(options),
        ignore_case: options.ignore.case,
        regex: options.ignore.regex.clone(),
    }) {
        Ok(builder) => Some(builder),
        Err(e) => {
            if e.invalid {
                warnings.push(Warning::new(WarningCode::InvalidRegex, None));
            }
            if e.limit {
                warnings.push(Warning::new(WarningCode::LimitExceeded, None));
            }
            None
        }
    }
}

/// True when the list holds an entry with severity `error`.
fn has_error(warnings: &[Warning]) -> bool {
    warnings
        .iter()
        .any(|w| w.code.severity() == diffyman_model::Severity::Error)
}

/// Sorts and deduplicates `warnings` (spec §10) and derives the status.
fn finish_warnings(warnings: &mut Vec<Warning>, changed: bool) -> Status {
    warnings.sort_by_key(Warning::sort_key);
    warnings.dedup_by(|a, b| a.sort_key() == b.sort_key());
    if has_error(warnings) {
        Status::Error
    } else if changed {
        Status::Different
    } else {
        Status::Identical
    }
}

/// A document with every block empty, to be filled in by one adapter.
fn empty_document(kind: Kind, left: FileMeta, right: FileMeta, options: &Options) -> DiffDocument {
    DiffDocument {
        schema_version: SCHEMA_VERSION.to_owned(),
        id: String::new(),
        kind,
        status: Status::Identical,
        left,
        right,
        position_encoding: options.position_encoding,
        stats: None,
        hunks: Vec::new(),
        moves: Vec::new(),
        structural: Vec::new(),
        table: None,
        regions: Vec::new(),
        bytes: Vec::new(),
        tree: None,
        warnings: Vec::new(),
        options: options.clone(),
        provenance: Provenance::default(),
        extensions: BTreeMap::new(),
    }
}

/// `kind: "text"`: the line diff (spec §7), and both decoded inputs when
/// they decode.
fn text_document<'a>(
    l: DecodedSide<'a>,
    r: DecodedSide<'a>,
    builder: Option<&KeyBuilder>,
    options: &Options,
    mut warnings: Vec<Warning>,
) -> (DiffDocument, Option<TextInputs<'a>>) {
    for (side, codes) in [(Side::Left, &l.codes), (Side::Right, &r.codes)] {
        for code in codes {
            warnings.push(Warning::new(*code, Some(side)));
        }
    }
    let (left_lines, right_lines) = (l.text.as_ref(), r.text.as_ref());
    let too_big = [left_lines, right_lines]
        .iter()
        .any(|s| s.is_some_and(|lines| lines.len() > line_diff::MAX_LINES))
        || !line_diff::within_size_limit(line_algorithm(options), l.meta.size, r.meta.size);
    if too_big {
        warnings.push(Warning::new(WarningCode::LimitExceeded, None));
    }

    let failed = has_error(&warnings);
    let (hunks, stats, moves) = match (left_lines, right_lines, builder) {
        (Some(lt), Some(rt), Some(builder)) if !failed => {
            let (hunks, stats, moves) = compare(
                TextLines {
                    text: lt.text(),
                    lines: lt.index(),
                },
                TextLines {
                    text: rt.text(),
                    lines: rt.index(),
                },
                builder,
                options,
                &mut warnings,
            );
            (hunks, Some(stats), moves)
        }
        _ => (Vec::new(), None, Vec::new()),
    };
    let changed = stats.as_ref().is_some_and(|s| s.change_count > 0);
    let status = finish_warnings(&mut warnings, changed);
    let mut doc = empty_document(Kind::Text, l.meta, r.meta, options);
    doc.status = status;
    doc.warnings = warnings;
    if status != Status::Error {
        doc.stats = stats;
        doc.hunks = hunks;
        doc.moves = moves;
    }
    let text = match (l.text, r.text) {
        (Some(left), Some(right)) => Some(TextInputs { left, right }),
        _ => None,
    };
    (doc, text)
}

/// `kind: "binary"`: changed byte ranges (spec §17). Text-only meta and
/// per-side text warnings do not apply.
fn binary_document(
    left: &[u8],
    right: &[u8],
    l: &DecodedSide<'_>,
    r: &DecodedSide<'_>,
    options: &Options,
    mut warnings: Vec<Warning>,
) -> DiffDocument {
    let binary_meta = |side: &DecodedSide<'_>| FileMeta {
        encoding: match side.detection.class {
            Class::Binary => None,
            _ => side.detection.encoding.map(|e| e.as_str().to_owned()),
        },
        line_terminator: None,
        final_newline: None,
        line_count: None,
        ..side.meta.clone()
    };
    let (left_meta, right_meta) = (binary_meta(l), binary_meta(r));
    if options.adapter == Adapter::Auto
        && (l.detection.class == Class::Text || r.detection.class == Class::Text)
    {
        warnings.push(Warning::new(WarningCode::BinaryFallback, None));
    }
    if !binary::within_size_limit(left_meta.size, right_meta.size) {
        warnings.push(Warning::new(WarningCode::LimitExceeded, None));
    }
    let diff = if has_error(&warnings) {
        None
    } else {
        Some(binary::diff_bytes(left, right))
    };
    if diff.as_ref().is_some_and(|d| d.coarse) {
        warnings.push(Warning::new(WarningCode::BytesCoarseLimit, None));
    }
    let changed = diff.as_ref().is_some_and(|d| !d.ranges.is_empty());
    let status = finish_warnings(&mut warnings, changed);
    let mut doc = empty_document(Kind::Binary, left_meta, right_meta, options);
    doc.status = status;
    doc.warnings = warnings;
    if let (Some(diff), true) = (diff, status != Status::Error) {
        let removed: u64 = diff.ranges.iter().map(|b| b.left_len).sum();
        let added: u64 = diff.ranges.iter().map(|b| b.right_len).sum();
        let (size_left, size_right) = (doc.left.size, doc.right.size);
        let unchanged = size_left - removed;
        doc.stats = Some(Stats {
            lines_left: 0,
            lines_right: 0,
            lines_added: 0,
            lines_removed: 0,
            lines_unchanged: 0,
            change_count: diff.ranges.len() as u64,
            hunk_count: 0,
            similarity: similarity(unchanged, size_left, size_right),
            bytes: Some(ByteStats {
                left: size_left,
                right: size_right,
                added,
                removed,
                unchanged,
            }),
            tree: None,
        });
        doc.bytes = diff
            .ranges
            .iter()
            .map(|b| ByteRange {
                left_offset: b.left_offset,
                left_length: b.left_len,
                right_offset: b.right_offset,
                right_length: b.right_len,
                op: match b.op() {
                    binary::RangeOp::Insert => OpKind::Insert,
                    binary::RangeOp::Delete => OpKind::Delete,
                    binary::RangeOp::Replace => OpKind::Replace,
                },
            })
            .collect();
    }
    doc
}

/// Strips a UTF-8 BOM and validates UTF-8. Returns `None` for invalid input.
pub fn decode_text(bytes: &[u8]) -> Option<&str> {
    let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    std::str::from_utf8(body).ok()
}

/// One detected and decoded input. `text` (the text after the BOM and its
/// lines) is `None` when the input is not decoded as text (not UTF-8, or not
/// needed for the adapter). The right side of byte-identical inputs shares
/// the left side's lines.
struct DecodedSide<'a> {
    meta: FileMeta,
    detection: Detection<'a>,
    text: Option<IndexedText<'a>>,
    /// Warnings about this side, in the order found.
    codes: Vec<WarningCode>,
}

impl DecodedSide<'_> {
    /// UTF-8 text or empty: the text adapter can compare it.
    fn is_utf8_text(&self) -> bool {
        match self.detection.class {
            Class::Empty => true,
            Class::Text => self.detection.encoding == Some(Encoding::Utf8),
            Class::Binary => false,
        }
    }
}

/// Detects and decodes both sides, on two threads where the platform has
/// them. The result does not depend on how the work is scheduled.
fn decode_pair<'a>(
    left: Input<'a>,
    right: Input<'a>,
    adapter: Adapter,
) -> (DecodedSide<'a>, DecodedSide<'a>) {
    /// Below this many bytes per side a thread costs more than it saves.
    #[cfg(not(target_family = "wasm"))]
    const PARALLEL_MIN_BYTES: usize = 4 << 20;
    #[cfg(not(target_family = "wasm"))]
    if left.bytes.len().min(right.bytes.len()) >= PARALLEL_MIN_BYTES {
        let decoded = std::thread::scope(|scope| {
            let handle = std::thread::Builder::new()
                .name("diffyman-decode".to_owned())
                .spawn_scoped(scope, || decode_side(right, adapter));
            let l = decode_side(left, adapter);
            let r = handle.ok().and_then(|h| h.join().ok());
            (l, r)
        });
        if let (l, Some(r)) = decoded {
            return (l, r);
        }
    }
    (decode_side(left, adapter), decode_side(right, adapter))
}

/// Detects one input and, when the text adapter may need it, splits it into
/// lines. Under `auto`, an input that is not UTF-8 text is not split: the
/// document will be binary.
fn decode_side(input: Input<'_>, adapter: Adapter) -> DecodedSide<'_> {
    let bytes = input.bytes;
    let detection = detect::detect(bytes, input.name);
    let bom = bytes.starts_with(b"\xEF\xBB\xBF");
    let mut codes = Vec::new();
    let mut meta = FileMeta {
        name: input.name.map(str::to_owned),
        media_type: Some(detection.media_type.to_owned()),
        size: bytes.len() as u64,
        sha256: diffyman_model::sha256_hex(bytes),
        encoding: None,
        bom,
        line_terminator: None,
        final_newline: None,
        line_count: None,
    };
    let wanted = match adapter {
        Adapter::Binary => false,
        Adapter::Text => true,
        _ => detection.class != Class::Binary,
    };
    let Some(text) = detection.utf8.filter(|_| wanted) else {
        if detection.utf8.is_none() {
            codes.push(WarningCode::InvalidUtf8);
        }
        return DecodedSide {
            meta,
            detection,
            text: None,
            codes,
        };
    };
    if bom {
        codes.push(WarningCode::BomStripped);
    }
    let indexed = IndexedText::new(text);
    let lines = indexed.index();
    let style = match lines.terminators().as_slice() {
        [] => LineTerminator::None,
        [Terminator::Lf] => LineTerminator::Lf,
        [Terminator::CrLf] => LineTerminator::Crlf,
        [Terminator::Cr] => LineTerminator::Cr,
        _ => LineTerminator::Mixed,
    };
    if style == LineTerminator::Mixed {
        codes.push(WarningCode::MixedLineEndings);
    }
    let last = lines.last_terminator();
    let final_newline = last.is_some_and(|t| !t.is_empty());
    if last.is_some_and(Terminator::is_empty) {
        codes.push(WarningCode::NoNewlineAtEof);
    }
    meta.encoding = Some("utf-8".to_owned());
    meta.line_terminator = Some(style);
    meta.final_newline = Some(final_newline);
    meta.line_count = Some(lines.len() as u64);
    DecodedSide {
        meta,
        detection,
        text: Some(indexed),
        codes,
    }
}

fn line_algorithm(options: &Options) -> LineAlgorithm {
    match options.algorithm {
        diffyman_model::Algorithm::Myers => LineAlgorithm::Myers,
        diffyman_model::Algorithm::Patience => LineAlgorithm::Patience,
        _ => LineAlgorithm::Histogram,
    }
}

fn whitespace_mode(options: &Options) -> key::WhitespaceMode {
    match options.ignore.whitespace {
        WhitespaceMode::Trailing => key::WhitespaceMode::Trailing,
        WhitespaceMode::Change => key::WhitespaceMode::Change,
        WhitespaceMode::All => key::WhitespaceMode::All,
        _ => key::WhitespaceMode::None,
    }
}

fn refine_options(options: &Options, builder: &KeyBuilder) -> Option<RefineOptions> {
    let granularity = match options.granularity {
        Granularity::Word => refine::Granularity::Word,
        Granularity::Char => refine::Granularity::Char,
        Granularity::Smart => refine::Granularity::Smart,
        _ => return None,
    };
    Some(RefineOptions {
        granularity,
        whitespace: whitespace_mode(options),
        ignore_case: options.ignore.case,
        position_encoding: match options.position_encoding {
            PositionEncoding::Utf16 => refine::PositionEncoding::Utf16,
            PositionEncoding::Codepoint => refine::PositionEncoding::Codepoint,
            _ => refine::PositionEncoding::Utf8,
        },
        masks: builder.masks().to_vec(),
    })
}

/// Runs the line diff, applies `ignore.blankLines`, groups hunks, detects
/// moves and computes stats.
///
/// The common prefix and suffix are trimmed before interning, and only the
/// lines in between are tokenized; the result is the same as diffing the
/// full token sequences (see `diffyman_algo::key::tokenize_core`).
fn compare(
    left: TextLines<'_>,
    right: TextLines<'_>,
    builder: &KeyBuilder,
    options: &Options,
    warnings: &mut Vec<Warning>,
) -> (Vec<Hunk>, Stats, Vec<Move>) {
    let core = key::tokenize_core(builder, left, right);
    let alignment = line_diff::align(
        line_algorithm(options),
        &core.left,
        &core.right,
        core.num_tokens,
    );
    if key::is_approximate_core(builder, left, right, &core, &alignment) {
        warnings.push(Warning::new(WarningCode::Approximate, None));
    }
    // Sliders go to their canonical position (spec §7.3.1). This may
    // shift runs into the trimmed prefix and suffix, so it works on the
    // whole-file alignment.
    let (compacted, _) = key::compact_core(builder, left, right, &core, &alignment);
    drop((core, alignment));
    let mut full = algo_ops::full_ops(&compacted);
    drop(compacted);
    if options.ignore.blank_lines {
        algo_ops::ignore_blank_changes_by(&mut full, content_of(left), content_of(right));
    }
    let context = usize::try_from(options.context).unwrap_or(usize::MAX);
    let groups = algo_ops::group_hunks(&full, context);

    // Moved-block detection runs before refinement: moved lines get no spans
    // on their ops (an edited move carries its own, below).
    // Lines of `ignored` ops are not changed lines, so they are never moved.
    // Without change ops there is nothing to move (and no limit to reach).
    let detected = if options.detect_moves && full.iter().any(|op| op.kind.is_change()) {
        // Only the lines of change ops are read, so memory follows the
        // changed lines, not the file length.
        let found = moves::detect_moves_by(content_of(left), content_of(right), &full, builder);
        if found.skipped_limit {
            warnings.push(Warning::new(WarningCode::MovesSkippedLimit, None));
        }
        found.moves
    } else {
        Vec::new()
    };
    let mut moved = Moved::default();
    if !detected.is_empty() {
        moved.left = vec![false; left.len()];
        moved.right = vec![false; right.len()];
    }
    for m in &detected {
        moved.left[m.left_start..m.left_start + m.left_len].fill(true);
        moved.right[m.right_start..m.right_start + m.right_len].fill(true);
    }

    let refine_opts = refine_options(options, builder);
    let mut skipped = false;
    let hunks: Vec<Hunk> = groups
        .iter()
        .enumerate()
        .map(|(i, group)| {
            let ops: Vec<Op> = group
                .iter()
                .map(|op| {
                    let inline = match (&refine_opts, op.kind) {
                        (Some(ro), algo_ops::OpKind::Replace) => {
                            inline_spans(op, left, right, &moved, ro, &mut skipped)
                        }
                        _ => Vec::new(),
                    };
                    to_model_op(op, inline)
                })
                .collect();
            let first = &ops[0];
            Hunk {
                id: format!("h{}", i + 1),
                left: Range {
                    start: first.left_range.start,
                    count: ops.iter().map(|o| o.left_range.count).sum(),
                },
                right: Range {
                    start: first.right_range.start,
                    count: ops.iter().map(|o| o.right_range.count).sum(),
                },
                ops,
            }
        })
        .collect();
    // Edited moves carry their own spans (spec §6.5); pure moves none.
    let move_inline: Vec<Vec<Span>> = detected
        .iter()
        .map(|m| match &refine_opts {
            Some(ro) => move_spans(m, left, right, ro, &mut skipped),
            None => Vec::new(),
        })
        .collect();
    if skipped {
        warnings.push(Warning::new(WarningCode::RefineSkippedLimit, None));
    }

    // Stats over the full op list (spec §6.3). Ignored lines are neither
    // changed nor unchanged, and they leave the similarity denominator.
    let (mut added, mut removed, mut unchanged, mut changes) = (0u64, 0u64, 0u64, 0u64);
    let (mut ignored_left, mut ignored_right) = (0u64, 0u64);
    for op in &full {
        match op.kind {
            algo_ops::OpKind::Equal => unchanged += op.left_len as u64,
            algo_ops::OpKind::Ignored => {
                ignored_left += op.left_len as u64;
                ignored_right += op.right_len as u64;
            }
            _ => {
                added += op.right_len as u64;
                removed += op.left_len as u64;
                changes += 1;
            }
        }
    }
    let lines_left = left.len() as u64;
    let lines_right = right.len() as u64;
    let stats = Stats {
        lines_left,
        lines_right,
        lines_added: added,
        lines_removed: removed,
        lines_unchanged: unchanged,
        change_count: changes,
        hunk_count: hunks.len() as u64,
        similarity: similarity(
            unchanged,
            lines_left - ignored_left,
            lines_right - ignored_right,
        ),
        bytes: None,
        tree: None,
    };
    let moves = to_model_moves(&detected, move_inline, &hunks);
    (hunks, stats, moves)
}

/// The content (without terminator) of each line of `side`, by 0-based number.
fn content_of<'t>(side: TextLines<'t>) -> impl Fn(usize) -> &'t str + Copy {
    move |i| side.line(i).content(side.text)
}

/// Which lines belong to a move (0-based), per side. Empty when there are
/// no moves.
#[derive(Default)]
struct Moved {
    left: Vec<bool>,
    right: Vec<bool>,
}

impl Moved {
    fn is_moved(flags: &[bool], line: usize) -> bool {
        flags.get(line).copied().unwrap_or(false)
    }
}

/// One op of the hunk list, flattened for range lookups (0-based lines).
struct FlatOp {
    hunk: usize,
    op: usize,
    left: (usize, usize),
    right: (usize, usize),
}

/// Maps detected moves onto `moves[]` (spec §6.5): 1-based ranges, ids in
/// array order, the ops each range touches, and each move's inline spans.
fn to_model_moves(
    detected: &[moves::DetectedMove],
    inline: Vec<Vec<Span>>,
    hunks: &[Hunk],
) -> Vec<Move> {
    let as_usize = |v: u64| usize::try_from(v).unwrap_or(usize::MAX);
    let flat: Vec<FlatOp> = hunks
        .iter()
        .enumerate()
        .flat_map(|(h, hunk)| {
            hunk.ops.iter().enumerate().map(move |(o, op)| FlatOp {
                hunk: h,
                op: o,
                left: (
                    as_usize(op.left_range.start) - 1,
                    as_usize(op.left_range.count),
                ),
                right: (
                    as_usize(op.right_range.start) - 1,
                    as_usize(op.right_range.count),
                ),
            })
        })
        .collect();
    // Ops are in document order, so both sides' ranges are sorted.
    let touching = |range: (usize, usize), side: fn(&FlatOp) -> (usize, usize)| -> Vec<OpRef> {
        let (start, end) = (range.0, range.0 + range.1);
        let first = flat.partition_point(|f| {
            let (s, n) = side(f);
            s + n <= start
        });
        flat[first..]
            .iter()
            .take_while(|f| side(f).0 < end)
            .filter(|f| side(f).1 > 0)
            .map(|f| OpRef {
                hunk: hunks[f.hunk].id.clone(),
                op: f.op as u64,
            })
            .collect()
    };
    detected
        .iter()
        .zip(inline)
        .enumerate()
        .map(|(i, (m, inline))| Move {
            id: format!("m{}", i + 1),
            kind: match m.kind {
                moves::MoveKind::Pure => MoveKind::Pure,
                moves::MoveKind::Edited => MoveKind::Edited,
            },
            from: MoveEnd {
                side: Side::Left,
                range: Range {
                    start: m.left_start as u64 + 1,
                    count: m.left_len as u64,
                },
            },
            to: MoveEnd {
                side: Side::Right,
                range: Range {
                    start: m.right_start as u64 + 1,
                    count: m.right_len as u64,
                },
            },
            similarity: Similarity::from_basis_points(m.similarity_bp),
            from_ops: touching((m.left_start, m.left_len), |f| f.left),
            to_ops: touching((m.right_start, m.right_len), |f| f.right),
            inline,
        })
        .collect()
}

/// `2U / (L + R)` rounded half up to 4 decimals, with integers only (spec §6.3).
/// `left` and `right` are the line counts without ignored lines.
fn similarity(unchanged: u64, left: u64, right: u64) -> Similarity {
    let total = u128::from(left) + u128::from(right);
    if total == 0 {
        return Similarity::ONE;
    }
    let n = (40_000 * u128::from(unchanged) + total) / (2 * total);
    Similarity::from_basis_points(u32::try_from(n).unwrap_or(10_000))
}

fn to_model_op(op: &LineOp, inline: Vec<Span>) -> Op {
    Op {
        op: match op.kind {
            algo_ops::OpKind::Equal => OpKind::Equal,
            algo_ops::OpKind::Insert => OpKind::Insert,
            algo_ops::OpKind::Delete => OpKind::Delete,
            algo_ops::OpKind::Replace => OpKind::Replace,
            algo_ops::OpKind::Ignored => OpKind::Ignored,
        },
        left_range: Range {
            start: op.left_start as u64 + 1,
            count: op.left_len as u64,
        },
        right_range: Range {
            start: op.right_start as u64 + 1,
            count: op.right_len as u64,
        },
        inline,
    }
}

/// Inline spans of one `replace` op. Lines that belong to a move are left out
/// of both token streams (moves spec §2), so they never carry spans.
fn inline_spans(
    op: &LineOp,
    left: TextLines<'_>,
    right: TextLines<'_>,
    moved: &Moved,
    options: &RefineOptions,
    skipped: &mut bool,
) -> Vec<Span> {
    let (l, l_index) = side_lines(left, op.left_start, op.left_len, |i| {
        !Moved::is_moved(&moved.left, i)
    });
    let (r, r_index) = side_lines(right, op.right_start, op.right_len, |i| {
        !Moved::is_moved(&moved.right, i)
    });
    model_spans(refine::refine(&l, &r, options), &l_index, &r_index, skipped)
}

/// Inline spans of an edited move (spec §6.5, moves spec §4.7): its two
/// ranges refined like one `replace` op, without the lines' indentation.
/// Pure moves get none. A move over the line limit is not read at all.
fn move_spans(
    m: &moves::DetectedMove,
    left: TextLines<'_>,
    right: TextLines<'_>,
    options: &RefineOptions,
    skipped: &mut bool,
) -> Vec<Span> {
    if m.kind != moves::MoveKind::Edited {
        return Vec::new();
    }
    if m.left_len > refine::REFINE_MAX_LINES || m.right_len > refine::REFINE_MAX_LINES {
        *skipped = true;
        return Vec::new();
    }
    let (l, l_index) = side_lines(left, m.left_start, m.left_len, |_| true);
    let (r, r_index) = side_lines(right, m.right_start, m.right_len, |_| true);
    model_spans(
        refine::refine_moved(&l, &r, options),
        &l_index,
        &r_index,
        skipped,
    )
}

/// The lines `start .. start + len` of `side` that `keep` accepts, with their
/// absolute 0-based line numbers.
fn side_lines(
    side: TextLines<'_>,
    start: usize,
    len: usize,
    keep: impl Fn(usize) -> bool,
) -> (Vec<SideLine<'_>>, Vec<usize>) {
    (start..start + len)
        .filter(|i| keep(*i))
        .map(|i| {
            let l = side.line(i);
            (
                SideLine {
                    content: l.content(side.text),
                    terminator: l.terminator,
                },
                i,
            )
        })
        .unzip()
}

/// Maps refined spans onto model spans with absolute 1-based line numbers;
/// `l_index` and `r_index` give the line number of each refined line.
fn model_spans(
    refined: Refined,
    l_index: &[usize],
    r_index: &[usize],
    skipped: &mut bool,
) -> Vec<Span> {
    match refined {
        Refined::SkippedLimit => {
            *skipped = true;
            Vec::new()
        }
        Refined::Spans(spans) => spans
            .into_iter()
            .map(|s| {
                let (side, line, kind) = match s.side {
                    SpanSide::Left => (Side::Left, l_index[s.line], OpKind::Delete),
                    SpanSide::Right => (Side::Right, r_index[s.line], OpKind::Insert),
                };
                Span {
                    side,
                    line: line as u64 + 1,
                    start: s.start,
                    end: s.end,
                    op: kind,
                }
            })
            .collect(),
    }
}

fn provenance(kind: Kind, options: &Options, settings: &Settings, total_micros: u64) -> Provenance {
    let mut extra = BTreeMap::new();
    let libraries: serde_json::Map<String, Value> = diffyman_algo::LIBRARY_VERSIONS
        .iter()
        .chain(&binary::LIBRARY_VERSIONS)
        .chain(TREE_LIBRARIES)
        .chain(&[("infer", detect::INFER_VERSION)])
        .map(|(name, version)| ((*name).to_owned(), Value::String((*version).to_owned())))
        .collect();
    extra.insert("libraries".to_owned(), Value::Object(libraries));
    extra.insert(
        "unicodeVersion".to_owned(),
        Value::String(diffyman_algo::segmentation_unicode_version()),
    );
    extra.insert(
        "detect".to_owned(),
        Value::String(detect::DETECT_ID.to_owned()),
    );
    let text = kind == Kind::Text;
    if text && options.granularity != Granularity::Line {
        extra.insert(
            "refine".to_owned(),
            Value::String(refine::REFINE_ID.to_owned()),
        );
    }
    if text && options.detect_moves {
        extra.insert(
            "moves".to_owned(),
            Value::String(moves::MOVES_ID.to_owned()),
        );
    }
    Provenance {
        engine: NameVersion {
            name: ENGINE_NAME.to_owned(),
            version: ENGINE_VERSION.to_owned(),
        },
        algorithm: match kind {
            Kind::Text => line_algorithm(options).id().to_owned(),
            #[cfg(feature = "archives")]
            Kind::Tree => tree::TREE_ID.to_owned(),
            _ => binary::BINARY_ID.to_owned(),
        },
        adapter: AdapterInfo {
            id: match kind {
                Kind::Text => "text",
                Kind::Tree => "tree",
                _ => "binary",
            }
            .to_owned(),
            version: ENGINE_VERSION.to_owned(),
        },
        binding: settings.binding.clone(),
        platform: Platform {
            // `consts::OS` is empty on wasm32-unknown-unknown; report the
            // target triple's OS field instead of an empty string.
            os: if std::env::consts::OS.is_empty() {
                "unknown".to_owned()
            } else {
                std::env::consts::OS.to_owned()
            },
            arch: std::env::consts::ARCH.to_owned(),
        },
        timing: Timing { total_micros },
        extra,
    }
}

/// Wall-clock timer; reads 0 where the platform has no clock (wasm32-unknown-unknown).
struct Timer {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    started: std::time::Instant,
}

impl Timer {
    fn start() -> Timer {
        Timer {
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            started: std::time::Instant::now(),
        }
    }

    #[allow(clippy::unused_self)]
    fn elapsed_micros(&self) -> u64 {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX)
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            0
        }
    }
}

#[cfg(test)]
mod tests;
