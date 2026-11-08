//! Diff Document types for diffyman, plus their canonical JSON form and JSON Schema.
//!
//! The normative definition is `docs/spec/diff-document.md` (schema `0.4.0`).
//! This crate holds no diff logic:
//!
//! - [`document`] and [`options`]: serde types for the document and its options.
//! - [`jcs`]: RFC 8785 canonical JSON, the byte-exact cross-language contract.
//! - [`compute_id`]: the document `id` (SHA-256 of the canonical form without
//!   `id` and `provenance`).
//! - [`document_schema`] and [`options_schema`]: JSON Schema 2020-12 generated
//!   from the types.
//! - [`lines`]: the line split of spec §7.2 (LF, CRLF, lone CR) that line
//!   positions in a document refer to, shared by the engine and renderers.

pub mod document;
pub mod jcs;
pub mod lines;
pub mod options;

pub use document::*;
pub use jcs::JcsError;
pub use options::*;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in &digest {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Computes the document `id`: `"sha256:"` + hex SHA-256 of the JCS bytes of the
/// document without its `id` and `provenance` keys (spec §5). The bytes are
/// hashed as they are produced, never held in memory.
pub fn compute_id(doc: &DiffDocument) -> Result<String, JcsError> {
    let hex = jcs::canonical_sha256_hex(doc, &["id", "provenance"])?;
    Ok(format!("sha256:{hex}"))
}

/// The canonical (JCS) JSON text of the whole document.
pub fn to_canonical_json(doc: &DiffDocument) -> Result<String, JcsError> {
    jcs::to_canonical_string(doc)
}

/// Writes the canonical (JCS) JSON text of the whole document to `writer`
/// as it is produced: the same bytes as [`to_canonical_json`], without
/// holding them. Pass a buffered writer.
pub fn write_canonical_json<W: std::io::Write>(
    doc: &DiffDocument,
    writer: W,
) -> Result<(), JcsError> {
    jcs::to_canonical_writer(doc, writer)
}

/// The canonical JSON text of the document without `provenance`: the form two
/// conformant engines must agree on byte for byte (spec §3).
pub fn to_conformance_json(doc: &DiffDocument) -> Result<String, JcsError> {
    jcs::to_canonical_string_without(doc, &["provenance"])
}

/// JSON Schema (2020-12) of the Diff Document as writers produce it: every
/// field required (spec §4), even though readers accept missing fields.
pub fn document_schema() -> Value {
    let schema = schemars::generate::SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<DiffDocument>();
    serde_json::to_value(schema).unwrap_or(Value::Null)
}

/// JSON Schema (2020-12) of the options input (`options.json`).
pub fn options_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(OptionsInput)).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn meta(name: &str) -> FileMeta {
        FileMeta {
            name: Some(name.to_owned()),
            media_type: Some("text/plain".to_owned()),
            size: 0,
            sha256: sha256_hex(b""),
            encoding: Some("utf-8".to_owned()),
            bom: false,
            line_terminator: Some(LineTerminator::None),
            final_newline: Some(false),
            line_count: Some(0),
        }
    }

    fn sample() -> DiffDocument {
        DiffDocument {
            schema_version: SCHEMA_VERSION.to_owned(),
            id: String::new(),
            kind: Kind::Text,
            status: Status::Identical,
            left: meta("a.txt"),
            right: meta("b.txt"),
            position_encoding: PositionEncoding::Utf8,
            stats: Some(Stats {
                lines_left: 0,
                lines_right: 0,
                lines_added: 0,
                lines_removed: 0,
                lines_unchanged: 0,
                change_count: 0,
                hunk_count: 0,
                similarity: Similarity::ONE,
                bytes: None,
                tree: None,
            }),
            hunks: vec![],
            moves: vec![],
            structural: vec![],
            table: None,
            regions: vec![],
            bytes: vec![],
            tree: None,
            warnings: vec![],
            options: Options::default(),
            provenance: Provenance {
                engine: NameVersion {
                    name: "diffyman".to_owned(),
                    version: "0.0.1".to_owned(),
                },
                algorithm: "text.histogram@1".to_owned(),
                adapter: AdapterInfo {
                    id: "text".to_owned(),
                    version: "0.0.1".to_owned(),
                },
                binding: "rust".to_owned(),
                platform: Platform {
                    os: "linux".to_owned(),
                    arch: "x86_64".to_owned(),
                },
                timing: Timing { total_micros: 1 },
                extra: BTreeMap::new(),
            },
            extensions: BTreeMap::new(),
        }
    }

    #[test]
    fn sha256_of_empty_input() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn id_ignores_provenance_and_id() {
        let a = sample();
        let mut b = sample();
        b.id = "sha256:whatever".to_owned();
        b.provenance.timing.total_micros = 999;
        b.provenance.binding = "cli".to_owned();
        assert_eq!(compute_id(&a).unwrap(), compute_id(&b).unwrap());
        assert!(compute_id(&a).unwrap().starts_with("sha256:"));
        let mut c = sample();
        c.left.name = Some("other.txt".to_owned());
        assert_ne!(compute_id(&a).unwrap(), compute_id(&c).unwrap());
    }

    /// The streamed id and writer give the bytes of the reference path:
    /// SHA-256 of the canonical string, and the canonical string itself.
    #[test]
    fn streamed_id_and_writer_match_the_reference_path() {
        let mut doc = sample();
        doc.warnings
            .push(Warning::new(WarningCode::NoNewlineAtEof, Some(Side::Left)));
        let text = jcs::to_canonical_string_without(&doc, &["id", "provenance"]).unwrap();
        assert_eq!(
            compute_id(&doc).unwrap(),
            format!("sha256:{}", sha256_hex(text.as_bytes()))
        );
        doc.id = compute_id(&doc).unwrap();
        let mut bytes = Vec::new();
        write_canonical_json(&doc, &mut bytes).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            to_canonical_json(&doc).unwrap()
        );
    }

    #[test]
    fn every_field_is_present_and_null_is_explicit() {
        let mut doc = sample();
        doc.stats = None;
        let value = serde_json::to_value(&doc).unwrap();
        let map = value.as_object().unwrap();
        for key in [
            "schemaVersion",
            "id",
            "kind",
            "status",
            "left",
            "right",
            "positionEncoding",
            "stats",
            "hunks",
            "moves",
            "structural",
            "table",
            "regions",
            "bytes",
            "tree",
            "warnings",
            "options",
            "provenance",
            "extensions",
        ] {
            assert!(map.contains_key(key), "missing {key}");
        }
        assert_eq!(map.len(), 19);
        assert!(map["stats"].is_null());
        assert!(map["table"].is_null());
        assert_eq!(map["left"].as_object().unwrap().len(), 9);
    }

    #[test]
    fn conformance_json_drops_provenance_only() {
        let doc = sample();
        let text = to_conformance_json(&doc).unwrap();
        assert!(!text.contains("provenance"));
        assert!(text.contains("\"id\":"));
        assert!(text.contains("\"similarity\":1"));
        let full = to_canonical_json(&doc).unwrap();
        assert!(full.contains("\"provenance\":{"));
    }

    #[test]
    fn document_round_trips() {
        let doc = sample();
        let text = to_canonical_json(&doc).unwrap();
        let back: DiffDocument = serde_json::from_str(&text).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn readers_ignore_unknown_fields_and_values() {
        let doc = sample();
        let mut value = serde_json::to_value(&doc).unwrap();
        let map = value.as_object_mut().unwrap();
        map.insert("futureField".to_owned(), serde_json::json!(1));
        map.insert("kind".to_owned(), serde_json::json!("hologram"));
        map["left"]["futureMeta"] = serde_json::json!({"a": 1});
        map["options"]["algorithm"] = serde_json::json!("diff3");
        map["options"]["futureOption"] = serde_json::json!(true);
        let back: DiffDocument = serde_json::from_value(value).unwrap();
        assert_eq!(back.kind, Kind::Unknown);
        assert_eq!(back.options.algorithm, Algorithm::Unknown);
        assert_eq!(back.left, doc.left);
    }

    #[test]
    fn readers_treat_missing_fields_as_null_or_empty() {
        let doc = sample();
        let mut value = serde_json::to_value(&doc).unwrap();
        let map = value.as_object_mut().unwrap();
        for key in ["stats", "hunks", "moves", "warnings", "extensions", "kind"] {
            map.remove(key);
        }
        map["left"].as_object_mut().unwrap().remove("lineCount");
        map["options"]
            .as_object_mut()
            .unwrap()
            .remove("granularity");
        map["options"]["ignore"]
            .as_object_mut()
            .unwrap()
            .remove("whitespace");
        let back: DiffDocument = serde_json::from_value(value).unwrap();
        assert_eq!(back.stats, None);
        assert!(back.hunks.is_empty() && back.moves.is_empty() && back.warnings.is_empty());
        assert!(back.extensions.is_empty());
        assert_eq!(back.kind, Kind::Unknown);
        assert_eq!(back.left.line_count, None);
        assert_eq!(back.left.sha256, doc.left.sha256);
        assert_eq!(back.options.granularity, Granularity::Unknown);
        assert_eq!(back.options.ignore.whitespace, WhitespaceMode::Unknown);
        assert_eq!(back.options.context, doc.options.context);
        assert_eq!(back.provenance, doc.provenance);

        // Even an empty object reads; nothing in it is a known value.
        let empty: DiffDocument = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.status, Status::Unknown);
        assert_eq!(empty.position_encoding, PositionEncoding::Unknown);
        assert_eq!(empty.options, Options::unset());
        assert!(empty.provenance.extra.is_empty());

        // Nested objects read the same way.
        let w: Warning = serde_json::from_str(r#"{"code": "bom-stripped"}"#).unwrap();
        assert_eq!(
            (w.code, w.severity, w.side),
            (WarningCode::BomStripped, Severity::Unknown, None)
        );
        let m: Move = serde_json::from_str(r#"{"id": "m1", "kind": "teleport"}"#).unwrap();
        assert_eq!(m.kind, MoveKind::Unknown);
        assert_eq!(m.from.side, Side::Unknown);
        assert!(m.from_ops.is_empty());
    }

    #[test]
    fn unknown_values_are_never_written() {
        assert!(serde_json::to_value(Kind::Unknown).is_err());
        assert!(serde_json::to_value(Side::Unknown).is_err());
        assert!(serde_json::to_value(WarningCode::Unknown).is_err());
        assert!(serde_json::to_value(Algorithm::Unknown).is_err());
        assert!(serde_json::to_value(PositionEncoding::Unknown).is_err());
        assert!(serde_json::to_value(Adapter::Unknown).is_err());
        let mut doc = sample();
        doc.options.granularity = Granularity::Unknown;
        assert!(to_canonical_json(&doc).is_err());
        assert!(compute_id(&doc).is_err());
    }

    #[test]
    fn move_shape_per_spec() {
        let m = Move {
            id: "m1".to_owned(),
            kind: MoveKind::Pure,
            from: MoveEnd {
                side: Side::Left,
                range: Range { start: 3, count: 4 },
            },
            to: MoveEnd {
                side: Side::Right,
                range: Range {
                    start: 10,
                    count: 4,
                },
            },
            similarity: Similarity::ONE,
            from_ops: vec![OpRef {
                hunk: "h1".to_owned(),
                op: 1,
            }],
            to_ops: vec![OpRef {
                hunk: "h2".to_owned(),
                op: 0,
            }],
            inline: Vec::new(),
        };
        assert_eq!(
            jcs::to_canonical_string(&m).unwrap(),
            concat!(
                r#"{"from":{"range":{"count":4,"start":3},"side":"left"},"#,
                r#""fromOps":[{"hunk":"h1","op":1}],"id":"m1","inline":[],"kind":"pure","#,
                r#""similarity":1,"to":{"range":{"count":4,"start":10},"side":"right"},"#,
                r#""toOps":[{"hunk":"h2","op":0}]}"#
            )
        );
        let edited = Move {
            kind: MoveKind::Edited,
            similarity: Similarity::from_basis_points(8750),
            inline: vec![
                Span {
                    side: Side::Left,
                    line: 4,
                    start: 6,
                    end: 9,
                    op: OpKind::Delete,
                },
                Span {
                    side: Side::Right,
                    line: 11,
                    start: 6,
                    end: 10,
                    op: OpKind::Insert,
                },
            ],
            ..m
        };
        let text = jcs::to_canonical_string(&edited).unwrap();
        assert!(text.contains(r#""similarity":0.875,"#), "{text}");
        assert!(
            text.contains(concat!(
                r#""inline":[{"end":9,"line":4,"op":"delete","side":"left","start":6},"#,
                r#"{"end":10,"line":11,"op":"insert","side":"right","start":6}],"#
            )),
            "{text}"
        );
        let back: Move = serde_json::from_str(&text).unwrap();
        assert_eq!(back, edited);
    }

    #[test]
    fn byte_range_and_byte_stats_shape_per_spec() {
        let range = ByteRange {
            left_offset: 10,
            left_length: 0,
            right_offset: 12,
            right_length: 3,
            op: OpKind::Insert,
        };
        assert_eq!(
            jcs::to_canonical_string(&range).unwrap(),
            r#"{"leftLength":0,"leftOffset":10,"op":"insert","rightLength":3,"rightOffset":12}"#
        );
        let stats = ByteStats {
            left: 10,
            right: 13,
            added: 3,
            removed: 0,
            unchanged: 10,
        };
        assert_eq!(
            jcs::to_canonical_string(&stats).unwrap(),
            r#"{"added":3,"left":10,"removed":0,"right":13,"unchanged":10}"#
        );
        let mut doc = sample();
        doc.kind = Kind::Binary;
        doc.bytes.push(range);
        doc.stats.as_mut().unwrap().bytes = Some(stats);
        let text = to_canonical_json(&doc).unwrap();
        assert!(text.contains(r#""kind":"binary""#), "{text}");
        let back: DiffDocument = serde_json::from_str(&text).unwrap();
        assert_eq!(back, doc);
        // A text document writes `stats.bytes: null`.
        assert!(
            to_canonical_json(&sample())
                .unwrap()
                .contains(r#""bytes":null"#)
        );
    }

    #[test]
    fn similarity_serialization() {
        let v = |bp| serde_json::to_value(Similarity::from_basis_points(bp)).unwrap();
        assert_eq!(jcs::canonicalize(&v(7273)).unwrap(), "0.7273");
        assert_eq!(jcs::canonicalize(&v(7500)).unwrap(), "0.75");
        assert_eq!(jcs::canonicalize(&v(10_000)).unwrap(), "1");
        assert_eq!(jcs::canonicalize(&v(0)).unwrap(), "0");
        assert_eq!(jcs::canonicalize(&v(1)).unwrap(), "0.0001");
        let back: Similarity = serde_json::from_str("0.7273").unwrap();
        assert_eq!(back.basis_points(), 7273);
    }

    #[test]
    fn warning_messages_and_order() {
        let w = Warning::new(WarningCode::NoNewlineAtEof, Some(Side::Right));
        assert_eq!(w.message, "No newline at end of file.");
        assert_eq!(w.severity, Severity::Warning);
        let text = jcs::to_canonical_string(&w).unwrap();
        assert_eq!(
            text,
            r#"{"code":"no-newline-at-eof","message":"No newline at end of file.","severity":"warning","side":"right"}"#
        );
        let e = Warning::new(WarningCode::InvalidUtf8, Some(Side::Left));
        assert_eq!(serde_json::to_value(e.code).unwrap(), "invalid-utf8");
        assert_eq!(e.severity, Severity::Error);
        let g = Warning::new(WarningCode::UnsupportedOption, None);
        let mut list = [g.clone(), w.clone(), e.clone()];
        list.sort_by_key(Warning::sort_key);
        assert_eq!(list, [e, w, g]);
    }

    #[test]
    fn warning_code_strings_match_serde() {
        for code in WarningCode::ALL {
            assert_eq!(serde_json::to_value(code).unwrap(), code.as_str());
            let back: WarningCode = serde_json::from_value(code.as_str().into()).unwrap();
            assert_eq!(back, code);
            assert_ne!(code.severity(), Severity::Unknown);
        }
    }

    /// The §10 table of `docs/spec/diff-document.md` is authoritative: every
    /// code, severity and message here must appear in it verbatim, and the
    /// table must list no other code.
    #[test]
    fn warning_table_matches_spec() {
        let spec = include_str!("../../../docs/spec/diff-document.md");
        let section = spec
            .split("## 10. Warnings and errors")
            .nth(1)
            .and_then(|s| s.split("\n## ").next())
            .expect("spec has section 10");
        let rows: Vec<Vec<&str>> = section
            .lines()
            .filter(|l| l.starts_with("| `"))
            .map(|l| l.trim_matches('|').split(" | ").map(str::trim).collect())
            .collect();
        assert_eq!(rows.len(), WarningCode::ALL.len(), "{rows:?}");
        for code in WarningCode::ALL {
            let name = format!("`{}`", code.as_str());
            let row = rows
                .iter()
                .find(|r| r[0] == name)
                .unwrap_or_else(|| panic!("{name} missing from spec §10"));
            let severity = match code.severity() {
                Severity::Error => "error",
                _ => "warning",
            };
            assert_eq!(row[1], severity, "{name}");
            assert_eq!(row[3], format!("`{}`", code.message()), "{name}");
        }
    }

    #[test]
    fn options_defaults_serialize_per_spec() {
        let text = jcs::to_canonical_string(&Options::default()).unwrap();
        assert_eq!(
            text,
            concat!(
                r#"{"adapter":"auto","algorithm":"histogram","context":3,"detectMoves":false,"granularity":"line","#,
                r#""ignore":{"blankLines":false,"case":false,"regex":[],"whitespace":"none"},"#,
                r#""positionEncoding":"utf-8","#,
                r#""tree":{"childBudget":16777216,"detectMoves":true,"exclude":[],"followSymlinks":false,"#,
                r#""ignoreFiles":false,"maxDepth":3}}"#
            )
        );
    }

    #[test]
    fn tree_shape_per_spec() {
        let mut child = sample();
        child.id = compute_id(&child).unwrap();
        let entry = TreeEntry {
            path: "a/b.txt".to_owned(),
            from_path: None,
            status: EntryStatus::Modified,
            left: Some(EntrySide {
                entry_type: EntryType::File,
                size: Some(3),
                sha256: Some("ab".to_owned()),
                target: None,
            }),
            right: None,
            child_document: Some(Box::new(ChildDocument::from(child.clone()))),
            child_ref: None,
        };
        let text = jcs::to_canonical_string(&entry).unwrap();
        assert!(
            text.starts_with(r#"{"childDocument":{"#),
            "keys sorted, child first: {text}"
        );
        assert!(
            text.ends_with(concat!(
                r#""childRef":null,"fromPath":null,"#,
                r#""left":{"sha256":"ab","size":3,"target":null,"type":"file"},"#,
                r#""path":"a/b.txt","right":null,"status":"modified"}"#
            )),
            "{text}"
        );
        // The child is the document without `provenance`, and its id is the
        // document's id.
        assert!(!text.contains("provenance"), "{text}");
        let child_text = jcs::to_canonical_string(&ChildDocument::from(child.clone())).unwrap();
        assert_eq!(
            child_text,
            jcs::to_canonical_string_without(&child, &["provenance"]).unwrap()
        );
        // Round trip, with an empty provenance on the way back.
        let back: TreeEntry = serde_json::from_str(&text).unwrap();
        assert_eq!(back, entry);
        let doc = DiffDocument::from(*back.child_document.unwrap());
        assert_eq!(doc.provenance, Provenance::default());
        assert_eq!(doc.id, child.id);

        let stats = TreeStats {
            entries_left: 3,
            entries_right: 2,
            added: 0,
            removed: 1,
            modified: 1,
            moved: 0,
            type_changed: 0,
            unchanged: 1,
        };
        assert_eq!(
            jcs::to_canonical_string(&stats).unwrap(),
            concat!(
                r#"{"added":0,"entriesLeft":3,"entriesRight":2,"modified":1,"moved":0,"#,
                r#""removed":1,"typeChanged":0,"unchanged":1}"#
            )
        );
        for (status, text) in [
            (EntryStatus::Added, "added"),
            (EntryStatus::Removed, "removed"),
            (EntryStatus::Modified, "modified"),
            (EntryStatus::Unchanged, "unchanged"),
            (EntryStatus::Moved, "moved"),
            (EntryStatus::TypeChange, "typeChange"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), text);
        }
        for (t, text) in [
            (EntryType::File, "file"),
            (EntryType::Directory, "directory"),
            (EntryType::Symlink, "symlink"),
            (EntryType::Hardlink, "hardlink"),
            (EntryType::Other, "other"),
        ] {
            assert_eq!(serde_json::to_value(t).unwrap(), text);
        }
        assert!(serde_json::to_value(EntryStatus::Unknown).is_err());
        assert!(serde_json::to_value(EntryType::Unknown).is_err());
        let unknown: EntrySide = serde_json::from_str(r#"{"type": "socketpair"}"#).unwrap();
        assert_eq!(unknown.entry_type, EntryType::Unknown);

        let mut doc = sample();
        doc.kind = Kind::Tree;
        doc.tree = Some(Tree {
            entries: vec![entry],
        });
        doc.stats.as_mut().unwrap().tree = Some(stats);
        let text = to_canonical_json(&doc).unwrap();
        let back: DiffDocument = serde_json::from_str(&text).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn options_input_rejects_unknown_keys() {
        assert!(serde_json::from_str::<OptionsInput>(r#"{"contxt": 1}"#).is_err());
        assert!(serde_json::from_str::<OptionsInput>(r#"{"ignore": {"x": 1}}"#).is_err());
        let ok: OptionsInput = serde_json::from_str(r#"{"context": 1}"#).unwrap();
        assert_eq!(ok.context, Some(1));
    }

    #[test]
    fn schemas_generate() {
        let doc = document_schema();
        assert_eq!(doc["title"], "DiffDocument");
        // Writers always emit every field (spec §4).
        assert_eq!(doc["required"].as_array().unwrap().len(), 19);
        // 0.4.0 adds `inline` to moves.
        assert_eq!(
            doc["$defs"]["Move"]["required"].as_array().unwrap().len(),
            8
        );
        let text = doc.to_string();
        assert!(text.contains("positionEncoding"));
        // 0.3.0 additions.
        assert!(
            text.contains("\"binary-fallback\""),
            "warning code `binary-fallback`"
        );
        assert!(text.contains("leftOffset"), "byte ranges");
        assert!(text.contains("\"adapter\""), "adapter option");
        assert!(text.contains("childDocument"), "tree entries");
        assert!(text.contains("\"typeChange\""), "entry status");
        assert!(text.contains("\"invalid-archive\""), "tree warning codes");
        assert_eq!(
            doc["$defs"]["TreeEntry"]["required"]
                .as_array()
                .unwrap()
                .len(),
            7
        );
        // 0.2.0 additions.
        assert!(text.contains("\"ignored\""), "op kind `ignored`");
        assert!(
            text.contains("\"invalid-regex\""),
            "warning code `invalid-regex`"
        );
        assert!(
            !text.contains("\"unknown\""),
            "unknown variants must stay out of the schema"
        );
        let opts = options_schema();
        assert_eq!(opts["additionalProperties"], false);
        assert!(
            !opts.to_string().contains("null"),
            "options never accept null"
        );
        assert_eq!(opts["properties"]["context"]["maximum"], 2_147_483_647);
        let tree = &opts["$defs"]["TreeOptionsInput"];
        assert_eq!(tree["additionalProperties"], false, "{tree}");
        assert_eq!(tree["properties"]["maxDepth"]["maximum"], 16);
    }
}
