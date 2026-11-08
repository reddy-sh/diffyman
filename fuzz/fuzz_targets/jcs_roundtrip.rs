//! The canonical JSON (RFC 8785) writer and the document reader.
//!
//! Two passes over each input:
//!
//! 1. The bytes as JSON text: when they parse, the tree writer
//!    (`jcs::canonicalize`) and the direct writer (`jcs::to_canonical_string`)
//!    agree, and canonical text parses back to the same canonical text.
//! 2. The bytes as the source of an arbitrary `DiffDocument`: the direct
//!    writer agrees with the tree writer (both fail or both give the same
//!    text), the text reads back as a document that writes the same bytes and
//!    has the same `id` and conformance form.
#![no_main]

use std::collections::BTreeMap;

use diffyman::model::{
    self, Adapter, AdapterInfo, Algorithm, ByteRange, ByteStats, ChildDocument, DiffDocument,
    EntrySide, EntryStatus, EntryType, FileMeta, Granularity, Hunk, IgnoreOptions, Kind,
    LineTerminator, Move, MoveEnd, MoveKind, NameVersion, Op, OpKind, OpRef, Options, Platform,
    PositionEncoding, Provenance, Range, Side, Similarity, Span, Stats, Status, Timing, Tree,
    TreeEntry, TreeOptions, TreeStats, Warning, WarningCode, WhitespaceMode, jcs,
};
use libfuzzer_sys::arbitrary::{Result, Unstructured};
use libfuzzer_sys::fuzz_target;
use serde_json::{Map, Number, Value};

fuzz_target!(|data: &[u8]| {
    if let Ok(value) = serde_json::from_slice::<Value>(data) {
        check_value(&value);
    }
    let mut u = Unstructured::new(data);
    if let Ok(doc) = document(&mut u) {
        check_doc(&doc);
    }
});

fn check_value(value: &Value) {
    let tree = jcs::canonicalize(value);
    let direct = jcs::to_canonical_string(value);
    match (&tree, &direct) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "tree and direct writers disagree"),
        (Err(_), Err(_)) => return,
        _ => panic!("one writer failed: tree {tree:?}, direct {direct:?}"),
    }
    let Ok(text) = tree else { return };
    let back: Value = serde_json::from_str(&text).expect("canonical text is JSON");
    // Floats that would print as integer literals beyond ±(2^53 − 1) are
    // rejected by the writers (spec §5), so canonical text is a fixed point.
    match jcs::canonicalize(&back) {
        Ok(again) => assert_eq!(again, text, "canonical text is not a fixed point"),
        Err(e) => panic!("canonical text does not canonicalize again: {e}"),
    }
}

fn check_doc(doc: &DiffDocument) {
    let direct = model::to_canonical_json(doc);
    let tree = serde_json::to_value(doc)
        .map_err(jcs::JcsError::from)
        .and_then(|v| jcs::canonicalize(&v));
    let text = match (direct, tree) {
        (Ok(a), Ok(b)) => {
            assert_eq!(a, b, "direct and tree writers disagree on a document");
            a
        }
        (Err(_), Err(_)) => return,
        (a, b) => panic!("one writer failed: direct {a:?}, tree {b:?}"),
    };
    let id = model::compute_id(doc).expect("id computes when the document serializes");
    let back: DiffDocument =
        serde_json::from_str(&text).expect("canonical text reads as a document");
    match model::to_canonical_json(&back) {
        Ok(again) => {
            assert_eq!(
                again, text,
                "document read/write round trip changed the bytes"
            );
            assert_eq!(
                model::compute_id(&back).expect("id"),
                id,
                "id changed in a round trip"
            );
            assert_eq!(
                model::to_conformance_json(&back).expect("conformance form"),
                model::to_conformance_json(doc).expect("conformance form"),
                "conformance form changed in a round trip"
            );
        }
        Err(e) => panic!("read-back document does not serialize: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Arbitrary documents. Unknown enum values are never generated: writers
// refuse them by design.
// ---------------------------------------------------------------------------

const MAX_ITEMS: usize = 4;

fn pick<T: Copy>(u: &mut Unstructured<'_>, items: &[T]) -> Result<T> {
    Ok(*u.choose(items)?)
}

/// Mostly small numbers; sometimes any `u64`, which may be outside the range
/// JCS allows and must then fail in both writers.
fn num(u: &mut Unstructured<'_>) -> Result<u64> {
    Ok(if u.ratio(1, 32)? {
        u.arbitrary::<u64>()?
    } else {
        u64::from(u.arbitrary::<u16>()?)
    })
}

fn string(u: &mut Unstructured<'_>) -> Result<String> {
    let len = u.int_in_range(0..=24)?;
    let mut s = String::new();
    for _ in 0..len {
        let c = if u.ratio(1, 4)? {
            u.arbitrary::<char>()?
        } else {
            char::from(u.int_in_range(0..=0x7f_u8)?)
        };
        s.push(c);
    }
    Ok(s)
}

fn list<T>(
    u: &mut Unstructured<'_>,
    mut f: impl FnMut(&mut Unstructured<'_>) -> Result<T>,
) -> Result<Vec<T>> {
    let n = u.int_in_range(0..=MAX_ITEMS)?;
    (0..n).map(|_| f(u)).collect()
}

fn json(u: &mut Unstructured<'_>, depth: u32) -> Result<Value> {
    let leaf = depth == 0;
    Ok(match u.int_in_range(0..=if leaf { 5 } else { 7 })? {
        0 => Value::Null,
        1 => Value::Bool(u.arbitrary()?),
        2 => Value::Number(Number::from(
            u.arbitrary::<i64>()? >> u.int_in_range(0..=63)?,
        )),
        3 => Value::Number(Number::from(num(u)?)),
        4 => Number::from_f64(f64::from_bits(u.arbitrary()?)).map_or(Value::Null, Value::Number),
        5 => Value::String(string(u)?),
        6 => Value::Array(list(u, |u| json(u, depth - 1))?),
        _ => {
            let mut map = Map::new();
            for _ in 0..u.int_in_range(0..=MAX_ITEMS)? {
                map.insert(string(u)?, json(u, depth - 1)?);
            }
            Value::Object(map)
        }
    })
}

fn range(u: &mut Unstructured<'_>) -> Result<Range> {
    Ok(Range {
        start: num(u)?,
        count: num(u)?,
    })
}

fn side(u: &mut Unstructured<'_>) -> Result<Side> {
    pick(u, &[Side::Left, Side::Right])
}

fn op_kind(u: &mut Unstructured<'_>) -> Result<OpKind> {
    pick(
        u,
        &[
            OpKind::Equal,
            OpKind::Insert,
            OpKind::Delete,
            OpKind::Replace,
            OpKind::Ignored,
        ],
    )
}

fn similarity(u: &mut Unstructured<'_>) -> Result<Similarity> {
    Ok(Similarity::from_basis_points(u.int_in_range(0..=10_000)?))
}

fn opt<T>(
    u: &mut Unstructured<'_>,
    f: impl FnOnce(&mut Unstructured<'_>) -> Result<T>,
) -> Result<Option<T>> {
    Ok(if u.arbitrary()? { Some(f(u)?) } else { None })
}

fn meta(u: &mut Unstructured<'_>) -> Result<FileMeta> {
    Ok(FileMeta {
        name: opt(u, string)?,
        media_type: opt(u, string)?,
        size: num(u)?,
        sha256: string(u)?,
        encoding: opt(u, string)?,
        bom: u.arbitrary()?,
        line_terminator: opt(u, |u| {
            pick(
                u,
                &[
                    LineTerminator::Lf,
                    LineTerminator::Crlf,
                    LineTerminator::Cr,
                    LineTerminator::Mixed,
                    LineTerminator::None,
                ],
            )
        })?,
        final_newline: opt(u, |u| u.arbitrary())?,
        line_count: opt(u, num)?,
    })
}

fn hunk(u: &mut Unstructured<'_>) -> Result<Hunk> {
    Ok(Hunk {
        id: string(u)?,
        left: range(u)?,
        right: range(u)?,
        ops: list(u, |u| {
            Ok(Op {
                op: op_kind(u)?,
                left_range: range(u)?,
                right_range: range(u)?,
                inline: list(u, |u| {
                    Ok(Span {
                        side: side(u)?,
                        line: num(u)?,
                        start: num(u)?,
                        end: num(u)?,
                        op: op_kind(u)?,
                    })
                })?,
            })
        })?,
    })
}

fn moved(u: &mut Unstructured<'_>) -> Result<Move> {
    let op_ref = |u: &mut Unstructured<'_>| -> Result<OpRef> {
        Ok(OpRef {
            hunk: string(u)?,
            op: num(u)?,
        })
    };
    Ok(Move {
        id: string(u)?,
        kind: pick(u, &[MoveKind::Pure, MoveKind::Edited])?,
        from: MoveEnd {
            side: side(u)?,
            range: range(u)?,
        },
        to: MoveEnd {
            side: side(u)?,
            range: range(u)?,
        },
        similarity: similarity(u)?,
        from_ops: list(u, op_ref)?,
        to_ops: list(u, op_ref)?,
        inline: list(u, |u| {
            Ok(Span {
                side: side(u)?,
                line: num(u)?,
                start: num(u)?,
                end: num(u)?,
                op: op_kind(u)?,
            })
        })?,
    })
}

fn options(u: &mut Unstructured<'_>) -> Result<Options> {
    Ok(Options {
        algorithm: pick(
            u,
            &[Algorithm::Histogram, Algorithm::Myers, Algorithm::Patience],
        )?,
        granularity: pick(
            u,
            &[
                Granularity::Line,
                Granularity::Word,
                Granularity::Char,
                Granularity::Smart,
            ],
        )?,
        ignore: IgnoreOptions {
            whitespace: pick(
                u,
                &[
                    WhitespaceMode::None,
                    WhitespaceMode::Trailing,
                    WhitespaceMode::Change,
                    WhitespaceMode::All,
                ],
            )?,
            case: u.arbitrary()?,
            blank_lines: u.arbitrary()?,
            regex: list(u, string)?,
        },
        context: u.arbitrary()?,
        position_encoding: encoding(u)?,
        detect_moves: u.arbitrary()?,
        adapter: pick(
            u,
            &[Adapter::Auto, Adapter::Text, Adapter::Binary, Adapter::Tree],
        )?,
        tree: TreeOptions {
            follow_symlinks: u.arbitrary()?,
            ignore_files: u.arbitrary()?,
            exclude: list(u, string)?,
            detect_moves: u.arbitrary()?,
            max_depth: u.arbitrary()?,
            child_budget: u.arbitrary()?,
        },
    })
}

fn entry_side(u: &mut Unstructured<'_>) -> Result<EntrySide> {
    Ok(EntrySide {
        entry_type: pick(
            u,
            &[
                EntryType::File,
                EntryType::Directory,
                EntryType::Symlink,
                EntryType::Hardlink,
                EntryType::Other,
            ],
        )?,
        size: opt(u, num)?,
        sha256: opt(u, string)?,
        target: opt(u, string)?,
    })
}

/// Entries of a tree; a child document nests at most `depth` more levels.
fn tree(u: &mut Unstructured<'_>, depth: u32) -> Result<Tree> {
    Ok(Tree {
        entries: list(u, |u| {
            Ok(TreeEntry {
                path: string(u)?,
                from_path: opt(u, string)?,
                status: pick(
                    u,
                    &[
                        EntryStatus::Added,
                        EntryStatus::Removed,
                        EntryStatus::Modified,
                        EntryStatus::Unchanged,
                        EntryStatus::Moved,
                        EntryStatus::TypeChange,
                    ],
                )?,
                left: opt(u, entry_side)?,
                right: opt(u, entry_side)?,
                child_document: if depth == 0 {
                    None
                } else {
                    opt(u, |u| {
                        document_at(u, depth - 1).map(|d| Box::new(ChildDocument::from(d)))
                    })?
                },
                child_ref: opt(u, string)?,
            })
        })?,
    })
}

fn byte_range(u: &mut Unstructured<'_>) -> Result<ByteRange> {
    Ok(ByteRange {
        left_offset: num(u)?,
        left_length: num(u)?,
        right_offset: num(u)?,
        right_length: num(u)?,
        op: op_kind(u)?,
    })
}

fn encoding(u: &mut Unstructured<'_>) -> Result<PositionEncoding> {
    pick(
        u,
        &[
            PositionEncoding::Utf8,
            PositionEncoding::Utf16,
            PositionEncoding::Codepoint,
        ],
    )
}

fn values(u: &mut Unstructured<'_>) -> Result<Vec<Value>> {
    list(u, |u| json(u, 3))
}

fn object(u: &mut Unstructured<'_>) -> Result<BTreeMap<String, Value>> {
    let mut map = BTreeMap::new();
    for _ in 0..u.int_in_range(0..=MAX_ITEMS)? {
        map.insert(string(u)?, json(u, 3)?);
    }
    Ok(map)
}

fn document(u: &mut Unstructured<'_>) -> Result<DiffDocument> {
    document_at(u, 2)
}

fn document_at(u: &mut Unstructured<'_>, depth: u32) -> Result<DiffDocument> {
    Ok(DiffDocument {
        schema_version: string(u)?,
        id: string(u)?,
        kind: pick(
            u,
            &[
                Kind::Text,
                Kind::Code,
                Kind::Structured,
                Kind::Table,
                Kind::Document,
                Kind::Image,
                Kind::Binary,
                Kind::Tree,
            ],
        )?,
        status: pick(
            u,
            &[
                Status::Identical,
                Status::Different,
                Status::Partial,
                Status::Error,
            ],
        )?,
        left: meta(u)?,
        right: meta(u)?,
        position_encoding: encoding(u)?,
        stats: opt(u, |u| {
            Ok(Stats {
                lines_left: num(u)?,
                lines_right: num(u)?,
                lines_added: num(u)?,
                lines_removed: num(u)?,
                lines_unchanged: num(u)?,
                change_count: num(u)?,
                hunk_count: num(u)?,
                similarity: similarity(u)?,
                bytes: opt(u, |u| {
                    Ok(ByteStats {
                        left: num(u)?,
                        right: num(u)?,
                        added: num(u)?,
                        removed: num(u)?,
                        unchanged: num(u)?,
                    })
                })?,
                tree: opt(u, |u| {
                    Ok(TreeStats {
                        entries_left: num(u)?,
                        entries_right: num(u)?,
                        added: num(u)?,
                        removed: num(u)?,
                        modified: num(u)?,
                        moved: num(u)?,
                        type_changed: num(u)?,
                        unchanged: num(u)?,
                    })
                })?,
            })
        })?,
        hunks: list(u, hunk)?,
        moves: list(u, moved)?,
        structural: values(u)?,
        table: opt(u, |u| json(u, 3))?,
        regions: values(u)?,
        bytes: list(u, byte_range)?,
        tree: opt(u, |u| tree(u, depth))?,
        warnings: list(u, |u| {
            let code = pick(u, &WarningCode::ALL)?;
            let side = opt(u, side)?;
            Ok(Warning::new(code, side))
        })?,
        options: options(u)?,
        provenance: Provenance {
            engine: NameVersion {
                name: string(u)?,
                version: string(u)?,
            },
            algorithm: string(u)?,
            adapter: AdapterInfo {
                id: string(u)?,
                version: string(u)?,
            },
            binding: string(u)?,
            platform: Platform {
                os: string(u)?,
                arch: string(u)?,
            },
            timing: Timing {
                total_micros: num(u)?,
            },
            extra: object(u)?,
        },
        extensions: object(u)?,
    })
}
