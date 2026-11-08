//! Diff Document types for diffyman, plus their canonical JSON form and JSON Schema.
//!
//! The normative definition is `docs/spec/diff-document.md` (schema `0.1.0`).
//! This crate holds no diff logic:
//!
//! - [`document`] and [`options`]: serde types for the document and its options.
//! - [`jcs`]: RFC 8785 canonical JSON, the byte-exact cross-language contract.
//! - [`compute_id`]: the document `id` (SHA-256 of the canonical form without
//!   `id` and `provenance`).
//! - [`document_schema`] and [`options_schema`]: JSON Schema 2020-12 generated
//!   from the types.

pub mod document;
pub mod jcs;
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

/// The document as a JSON value with the top-level keys in `remove` taken out.
fn value_without(doc: &DiffDocument, remove: &[&str]) -> Result<Value, JcsError> {
    let mut value = serde_json::to_value(doc)?;
    if let Value::Object(map) = &mut value {
        for key in remove {
            map.remove(*key);
        }
    }
    Ok(value)
}

/// Computes the document `id`: `"sha256:"` + hex SHA-256 of the JCS bytes of the
/// document without its `id` and `provenance` keys (spec §5).
pub fn compute_id(doc: &DiffDocument) -> Result<String, JcsError> {
    let canonical = jcs::canonicalize(&value_without(doc, &["id", "provenance"])?)?;
    Ok(format!("sha256:{}", sha256_hex(canonical.as_bytes())))
}

/// The canonical (JCS) JSON text of the whole document.
pub fn to_canonical_json(doc: &DiffDocument) -> Result<String, JcsError> {
    jcs::to_canonical_string(doc)
}

/// The canonical JSON text of the document without `provenance`: the form two
/// conformant engines must agree on byte for byte (spec §3).
pub fn to_conformance_json(doc: &DiffDocument) -> Result<String, JcsError> {
    jcs::canonicalize(&value_without(doc, &["provenance"])?)
}

/// JSON Schema (2020-12) of the Diff Document.
pub fn document_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(DiffDocument)).unwrap_or(Value::Null)
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
        let back: DiffDocument = serde_json::from_value(value).unwrap();
        assert_eq!(back.kind, Kind::Unknown);
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
        for code in [
            WarningCode::BomStripped,
            WarningCode::MixedLineEndings,
            WarningCode::NoNewlineAtEof,
            WarningCode::Approximate,
            WarningCode::RefineSkippedLimit,
            WarningCode::InvalidUtf8,
            WarningCode::UnsupportedOption,
            WarningCode::LimitExceeded,
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), code.as_str());
        }
    }

    #[test]
    fn options_defaults_serialize_per_spec() {
        let text = jcs::to_canonical_string(&Options::default()).unwrap();
        assert_eq!(
            text,
            concat!(
                r#"{"algorithm":"histogram","context":3,"detectMoves":false,"granularity":"line","#,
                r#""ignore":{"blankLines":false,"case":false,"regex":[],"whitespace":"none"},"#,
                r#""positionEncoding":"utf-8"}"#
            )
        );
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
        let text = doc.to_string();
        assert!(text.contains("positionEncoding"));
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
    }
}
