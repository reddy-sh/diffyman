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
//! Behavior follows `docs/spec/diff-document.md` (schema `0.2.0`):
//!
//! - A UTF-8 BOM is removed before comparison (`bom-stripped`).
//! - Lines end with LF, CRLF or a lone CR. A missing final newline is reported
//!   (`no-newline-at-eof`) and, under whitespace mode `none`, compares as a change.
//! - Invalid UTF-8 gives `status: "error"` with an `invalid-utf8` error; there is
//!   no binary fallback yet.
//! - `ignore.regex` masks matches before comparison; `ignore.blankLines` turns
//!   blank-only changes into `ignored` ops, which are not changes.
//! - Invalid options are rejected with [`Error::InvalidOptions`]. A regex that
//!   does not compile, or is over a limit, gives a document with an
//!   `invalid-regex` or `limit-exceeded` error.
//! - Timing appears only in `provenance`.

mod options;

use std::collections::BTreeMap;

pub use diffyman_algo::lines::{Line, Terminator, split_lines};
pub use diffyman_model as model;
use diffyman_model::{
    AdapterInfo, DiffDocument, FileMeta, Granularity, Hunk, JcsError, Kind, LineTerminator, Move,
    MoveEnd, MoveKind, NameVersion, Op, OpKind, OpRef, Options, Platform, PositionEncoding,
    Provenance, Range, SCHEMA_VERSION, Side, Similarity, Span, Stats, Status, Timing, Warning,
    WarningCode, WhitespaceMode,
};
pub use options::{parse_options, parse_options_value, resolve_options, validate_options};
use serde_json::Value;

use diffyman_algo::key::{self, Interner, KeyBuilder, KeyOptions};
use diffyman_algo::line_diff::{self, LineAlgorithm};
use diffyman_algo::moves::{self, MoveOptions};
use diffyman_algo::ops::{self as algo_ops, LineOp};
use diffyman_algo::refine::{self, RefineOptions, Refined, SideLine, SpanSide};

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
    let timer = Timer::start();
    let mut warnings = Vec::new();
    let l = decode_side(left, Side::Left, &mut warnings);
    let r = decode_side(right, Side::Right, &mut warnings);
    let builder = match KeyBuilder::new(&KeyOptions {
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
    };
    let too_big =
        [&l, &r].iter().any(|s| {
            s.lines
                .as_ref()
                .is_some_and(|lines| lines.len() > line_diff::MAX_LINES)
        }) || !line_diff::within_size_limit(line_algorithm(options), l.meta.size, r.meta.size);
    if too_big {
        warnings.push(Warning::new(WarningCode::LimitExceeded, None));
    }

    let failed = warnings
        .iter()
        .any(|w| w.code.severity() == diffyman_model::Severity::Error);
    let (hunks, stats, moves) = match (&l.text, &l.lines, &r.text, &r.lines, &builder) {
        (Some(lt), Some(ll), Some(rt), Some(rl), Some(builder)) if !failed => {
            let (hunks, stats, moves) = compare(
                Sides {
                    left_text: lt,
                    left: ll,
                    right_text: rt,
                    right: rl,
                },
                builder,
                options,
                &mut warnings,
            );
            (hunks, Some(stats), moves)
        }
        _ => (Vec::new(), None, Vec::new()),
    };
    warnings.sort_by_key(Warning::sort_key);
    warnings.dedup_by(|a, b| a.sort_key() == b.sort_key());
    let status = if warnings
        .iter()
        .any(|w| w.code.severity() == diffyman_model::Severity::Error)
    {
        Status::Error
    } else if stats.as_ref().is_some_and(|s| s.change_count > 0) {
        Status::Different
    } else {
        Status::Identical
    };
    let (hunks, stats, moves) = if status == Status::Error {
        (Vec::new(), None, Vec::new())
    } else {
        (hunks, stats, moves)
    };

    let mut doc = DiffDocument {
        schema_version: SCHEMA_VERSION.to_owned(),
        id: String::new(),
        kind: Kind::Text,
        status,
        left: l.meta,
        right: r.meta,
        position_encoding: options.position_encoding,
        stats,
        hunks,
        moves,
        structural: Vec::new(),
        table: None,
        regions: Vec::new(),
        bytes: Vec::new(),
        tree: None,
        warnings,
        options: options.clone(),
        provenance: provenance(options, settings, 0),
        extensions: BTreeMap::new(),
    };
    doc.id = diffyman_model::compute_id(&doc)?;
    doc.provenance.timing.total_micros = timer.elapsed_micros();
    Ok(doc)
}

/// Strips a UTF-8 BOM and validates UTF-8. Returns `None` for invalid input.
pub fn decode_text(bytes: &[u8]) -> Option<&str> {
    let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    std::str::from_utf8(body).ok()
}

struct DecodedSide<'a> {
    meta: FileMeta,
    text: Option<&'a str>,
    lines: Option<Vec<Line>>,
}

fn decode_side<'a>(input: Input<'a>, side: Side, warnings: &mut Vec<Warning>) -> DecodedSide<'a> {
    let bytes = input.bytes;
    let bom = bytes.starts_with(b"\xEF\xBB\xBF");
    let mut meta = FileMeta {
        name: input.name.map(str::to_owned),
        media_type: Some("text/plain".to_owned()),
        size: bytes.len() as u64,
        sha256: diffyman_model::sha256_hex(bytes),
        encoding: None,
        bom,
        line_terminator: None,
        final_newline: None,
        line_count: None,
    };
    let Some(text) = decode_text(bytes) else {
        warnings.push(Warning::new(WarningCode::InvalidUtf8, Some(side)));
        return DecodedSide {
            meta,
            text: None,
            lines: None,
        };
    };
    if bom {
        warnings.push(Warning::new(WarningCode::BomStripped, Some(side)));
    }
    let lines = split_lines(text);
    let mut styles: Vec<Terminator> = Vec::new();
    for line in &lines {
        if !line.terminator.is_empty() && !styles.contains(&line.terminator) {
            styles.push(line.terminator);
        }
    }
    let style = match styles.as_slice() {
        [] => LineTerminator::None,
        [Terminator::Lf] => LineTerminator::Lf,
        [Terminator::CrLf] => LineTerminator::Crlf,
        [Terminator::Cr] => LineTerminator::Cr,
        _ => LineTerminator::Mixed,
    };
    if style == LineTerminator::Mixed {
        warnings.push(Warning::new(WarningCode::MixedLineEndings, Some(side)));
    }
    let final_newline = lines.last().is_some_and(|l| !l.terminator.is_empty());
    if lines.last().is_some_and(|l| l.terminator.is_empty()) {
        warnings.push(Warning::new(WarningCode::NoNewlineAtEof, Some(side)));
    }
    meta.encoding = Some("utf-8".to_owned());
    meta.line_terminator = Some(style);
    meta.final_newline = Some(final_newline);
    meta.line_count = Some(lines.len() as u64);
    DecodedSide {
        meta,
        text: Some(text),
        lines: Some(lines),
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

fn refine_options(options: &Options) -> Option<RefineOptions> {
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
    })
}

/// Both decoded inputs and their lines.
#[derive(Clone, Copy)]
struct Sides<'a> {
    left_text: &'a str,
    left: &'a [Line],
    right_text: &'a str,
    right: &'a [Line],
}

/// Runs the line diff, applies `ignore.blankLines`, groups hunks, detects
/// moves and computes stats.
fn compare(
    sides: Sides<'_>,
    builder: &KeyBuilder,
    options: &Options,
    warnings: &mut Vec<Warning>,
) -> (Vec<Hunk>, Stats, Vec<Move>) {
    let Sides {
        left_text,
        left,
        right_text,
        right,
    } = sides;
    let mut interner = Interner::new();
    let (lt, rt) = key::intern_sides(builder, &mut interner, left_text, left, right_text, right);
    let alignment = line_diff::align(line_algorithm(options), &lt, &rt, interner.num_tokens());
    if line_diff::is_approximate(&lt, &rt, &alignment) {
        warnings.push(Warning::new(WarningCode::Approximate, None));
    }
    let mut full = algo_ops::full_ops(&alignment);
    let contents = (options.ignore.blank_lines || options.detect_moves).then(|| {
        let lc: Vec<&str> = left.iter().map(|l| l.content(left_text)).collect();
        let rc: Vec<&str> = right.iter().map(|l| l.content(right_text)).collect();
        (lc, rc)
    });
    if options.ignore.blank_lines
        && let Some((lc, rc)) = &contents
    {
        algo_ops::ignore_blank_changes(&mut full, lc, rc);
    }
    let context = usize::try_from(options.context).unwrap_or(usize::MAX);
    let groups = algo_ops::group_hunks(&full, context);

    // Moved-block detection runs before refinement: moved lines get no spans.
    // Lines of `ignored` ops are not changed lines, so they are never moved.
    let detected = if options.detect_moves
        && let Some((lc, rc)) = &contents
    {
        let found = moves::detect_moves(
            lc,
            rc,
            &full,
            &MoveOptions {
                whitespace: whitespace_mode(options),
                ignore_case: options.ignore.case,
            },
        );
        if found.skipped_limit {
            warnings.push(Warning::new(WarningCode::MovesSkippedLimit, None));
        }
        found.moves
    } else {
        Vec::new()
    };
    let mut moved = Moved {
        left: vec![false; left.len()],
        right: vec![false; right.len()],
    };
    for m in &detected {
        moved.left[m.left_start..m.left_start + m.left_len].fill(true);
        moved.right[m.right_start..m.right_start + m.right_len].fill(true);
    }

    let refine_opts = refine_options(options);
    let mut skipped = false;
    let hunks: Vec<Hunk> = groups
        .iter()
        .enumerate()
        .map(|(i, group)| {
            let ops: Vec<Op> = group
                .iter()
                .map(|op| {
                    let inline = match (&refine_opts, op.kind) {
                        (Some(ro), algo_ops::OpKind::Replace) => inline_spans(
                            op,
                            (left_text, left),
                            (right_text, right),
                            &moved,
                            ro,
                            &mut skipped,
                        ),
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
    };
    let moves = to_model_moves(&detected, &hunks);
    (hunks, stats, moves)
}

/// Which lines belong to a move (0-based), per side.
struct Moved {
    left: Vec<bool>,
    right: Vec<bool>,
}

/// One op of the hunk list, flattened for range lookups (0-based lines).
struct FlatOp {
    hunk: usize,
    op: usize,
    left: (usize, usize),
    right: (usize, usize),
}

/// Maps detected moves onto `moves[]` (spec §6.5): 1-based ranges, ids in
/// array order, and the ops each range touches.
fn to_model_moves(detected: &[moves::DetectedMove], hunks: &[Hunk]) -> Vec<Move> {
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
        .enumerate()
        .map(|(i, m)| Move {
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
    left: (&str, &[Line]),
    right: (&str, &[Line]),
    moved: &Moved,
    options: &RefineOptions,
    skipped: &mut bool,
) -> Vec<Span> {
    /// The op side's unmoved lines and their absolute 0-based line numbers.
    fn side_lines<'t>(
        (text, lines): (&'t str, &[Line]),
        moved: &[bool],
        start: usize,
        len: usize,
    ) -> (Vec<SideLine<'t>>, Vec<usize>) {
        (start..start + len)
            .filter(|i| !moved[*i])
            .map(|i| {
                let l = lines[i];
                (
                    SideLine {
                        content: &text[l.start..l.end],
                        terminator: l.terminator,
                    },
                    i,
                )
            })
            .unzip()
    }
    let (l, l_index) = side_lines(left, &moved.left, op.left_start, op.left_len);
    let (r, r_index) = side_lines(right, &moved.right, op.right_start, op.right_len);
    match refine::refine(&l, &r, options) {
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

fn provenance(options: &Options, settings: &Settings, total_micros: u64) -> Provenance {
    let mut extra = BTreeMap::new();
    let libraries: serde_json::Map<String, Value> = diffyman_algo::LIBRARY_VERSIONS
        .iter()
        .map(|(name, version)| ((*name).to_owned(), Value::String((*version).to_owned())))
        .collect();
    extra.insert("libraries".to_owned(), Value::Object(libraries));
    extra.insert(
        "unicodeVersion".to_owned(),
        Value::String(diffyman_algo::segmentation_unicode_version()),
    );
    if options.granularity != Granularity::Line {
        extra.insert(
            "refine".to_owned(),
            Value::String(refine::REFINE_ID.to_owned()),
        );
    }
    if options.detect_moves {
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
        algorithm: line_algorithm(options).id().to_owned(),
        adapter: AdapterInfo {
            id: "text".to_owned(),
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
