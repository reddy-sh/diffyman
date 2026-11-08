use std::fmt::Write as _;

use super::*;
use serde_json::json;

const LEFT: &[u8] = b"apple\nbanana\ncherry\ndate\nelderberry\n";
const RIGHT: &[u8] = b"apple\nblueberry\ncherry\ndate\nelderberry\nfig";

fn doc(left: &[u8], right: &[u8], options: &str) -> DiffDocument {
    let options = parse_options(options).unwrap();
    diff_document(
        Input::named(left, "left.txt"),
        Input::named(right, "right.txt"),
        &options,
    )
    .unwrap()
}

fn codes(d: &DiffDocument) -> Vec<(&'static str, Option<Side>)> {
    d.warnings
        .iter()
        .map(|w| (w.code.as_str(), w.side))
        .collect()
}

/// Diff Document spec §14: the worked example, including its `id`.
#[test]
fn worked_example_matches_spec() {
    let d = doc(LEFT, RIGHT, r#"{"context": 1}"#);
    assert_eq!(d.left.size, 36);
    assert_eq!(
        d.left.sha256,
        "9832a4e44238cd39a2b5561f8cce7a35e2e8d4b86a444f18a1ee260941f7bfdc"
    );
    assert_eq!(d.right.size, 42);
    assert_eq!(
        d.right.sha256,
        "2ae70b2ff240a155ac5950ab03bdd1a37b2195c15916cf338ce8bf600254095d"
    );
    let value = serde_json::to_value(&d).unwrap();
    let expected = json!({
        "schemaVersion": "0.4.0",
        "kind": "text",
        "status": "different",
        "positionEncoding": "utf-8",
        "stats": {
            "linesLeft": 5, "linesRight": 6, "linesAdded": 2, "linesRemoved": 1,
            "linesUnchanged": 4, "changeCount": 2, "hunkCount": 2, "similarity": 0.7273,
            "bytes": null, "tree": null
        },
        "hunks": [
            {
                "id": "h1",
                "left": { "start": 1, "count": 3 },
                "right": { "start": 1, "count": 3 },
                "ops": [
                    { "op": "equal",   "leftRange": { "start": 1, "count": 1 }, "rightRange": { "start": 1, "count": 1 }, "inline": [] },
                    { "op": "replace", "leftRange": { "start": 2, "count": 1 }, "rightRange": { "start": 2, "count": 1 }, "inline": [] },
                    { "op": "equal",   "leftRange": { "start": 3, "count": 1 }, "rightRange": { "start": 3, "count": 1 }, "inline": [] }
                ]
            },
            {
                "id": "h2",
                "left": { "start": 5, "count": 1 },
                "right": { "start": 5, "count": 2 },
                "ops": [
                    { "op": "equal",  "leftRange": { "start": 5, "count": 1 }, "rightRange": { "start": 5, "count": 1 }, "inline": [] },
                    { "op": "insert", "leftRange": { "start": 6, "count": 0 }, "rightRange": { "start": 6, "count": 1 }, "inline": [] }
                ]
            }
        ],
        "warnings": [
            { "code": "no-newline-at-eof", "severity": "warning", "side": "right", "message": "No newline at end of file." }
        ],
        "options": {
            "algorithm": "histogram",
            "granularity": "line",
            "ignore": { "whitespace": "none", "case": false, "blankLines": false, "regex": [] },
            "context": 1,
            "positionEncoding": "utf-8",
            "detectMoves": false,
            "adapter": "auto",
            "tree": {
                "followSymlinks": false, "ignoreFiles": false, "exclude": [],
                "detectMoves": true, "maxDepth": 3, "childBudget": 16_777_216
            }
        },
        "bytes": [],
        "tree": null
    });
    for (key, want) in expected.as_object().unwrap() {
        assert_eq!(&value[key], want, "field {key}");
    }
    assert_eq!(value["left"]["lineTerminator"], "lf");
    assert_eq!(value["right"]["finalNewline"], false);
    assert_eq!(value["right"]["lineCount"], 6);
    assert_eq!(value["left"]["mediaType"], "text/plain");
    assert_eq!(
        d.id,
        "sha256:f221a785d0e51a6a0ab0887619d144522fd921910db3b2460b4865712f8de99e"
    );
}

#[test]
fn binding_settings_change_only_provenance() {
    let settings = Settings {
        binding: "python".to_owned(),
    };
    let left = Input::named(LEFT, "left.txt");
    let right = Input::named(RIGHT, "right.txt");
    let a = diff_inputs_with(left, right, "{}", &settings).unwrap();
    let b = diff_inputs(left, right, "{}").unwrap();
    let mut va: Value = serde_json::from_str(&a).unwrap();
    let mut vb: Value = serde_json::from_str(&b).unwrap();
    assert_eq!(va["provenance"]["binding"], "python");
    assert_eq!(vb["provenance"]["binding"], "rust");
    va.as_object_mut().unwrap().remove("provenance");
    vb.as_object_mut().unwrap().remove("provenance");
    assert_eq!(va, vb);
    assert!(matches!(
        diff_inputs_with(left, right, r#"{"nope": 1}"#, &settings),
        Err(Error::InvalidOptions(_))
    ));
}

#[test]
fn canonical_output_is_stable_and_provenance_is_excluded() {
    let a = diff_inputs(
        Input::named(LEFT, "dir/left.txt"),
        Input::named(RIGHT, "right.txt"),
        "{}",
    )
    .unwrap();
    let value: Value = serde_json::from_str(&a).unwrap();
    assert_eq!(value["left"]["name"], "left.txt");
    assert_eq!(value["provenance"]["engine"]["name"], "diffyman");
    assert_eq!(value["provenance"]["algorithm"], "text.histogram@1");
    assert_eq!(value["provenance"]["binding"], "rust");
    // Canonical text re-canonicalizes to itself.
    assert_eq!(diffyman_model::jcs::canonicalize(&value).unwrap(), a);
    let d1 = doc(LEFT, RIGHT, "{}");
    let d2 = doc(LEFT, RIGHT, "{}");
    assert_eq!(
        diffyman_model::to_conformance_json(&d1).unwrap(),
        diffyman_model::to_conformance_json(&d2).unwrap()
    );
}

#[test]
fn unnamed_inputs_have_null_names() {
    let json = diff(b"a\n", b"a\n", "").unwrap();
    let value: Value = serde_json::from_str(&json).unwrap();
    assert!(value["left"]["name"].is_null());
    assert_eq!(value["status"], "identical");
    assert_eq!(value["stats"]["similarity"], 1);
    assert_eq!(value["hunks"], json!([]));
}

#[test]
fn empty_files() {
    let d = doc(b"", b"", "{}");
    assert_eq!(d.status, Status::Identical);
    assert_eq!(d.left.line_count, Some(0));
    assert_eq!(d.left.line_terminator, Some(LineTerminator::None));
    assert_eq!(d.left.final_newline, Some(false));
    assert_eq!(d.stats.unwrap().similarity, Similarity::ONE);
    let d = doc(b"", b"x\n", "{}");
    let op = &d.hunks[0].ops[0];
    assert_eq!(op.op, OpKind::Insert);
    assert_eq!(op.left_range, Range { start: 1, count: 0 });
    assert_eq!(
        d.stats.unwrap().similarity,
        Similarity::from_basis_points(0)
    );
}

#[test]
fn bom_is_stripped_and_identical() {
    let d = doc(b"\xEF\xBB\xBFa\nb\n", b"a\nb\n", "{}");
    assert_eq!(d.status, Status::Identical);
    assert!(d.left.bom);
    assert_eq!(d.left.size, 7);
    assert_ne!(d.left.sha256, d.right.sha256);
    assert_eq!(codes(&d), [("bom-stripped", Some(Side::Left))]);
}

#[test]
fn line_terminators() {
    let d = doc(b"a\r\nb\r\n", b"a\nb\n", "{}");
    assert_eq!(d.left.line_terminator, Some(LineTerminator::Crlf));
    assert_eq!(d.status, Status::Different);
    let d = doc(
        b"a\r\nb\r\n",
        b"a\nb\n",
        r#"{"ignore": {"whitespace": "trailing"}}"#,
    );
    assert_eq!(d.status, Status::Identical);
    let d = doc(b"a\rb\r", b"a\rb\r", "{}");
    assert_eq!(d.left.line_terminator, Some(LineTerminator::Cr));
    assert_eq!(d.left.line_count, Some(2));
    let d = doc(b"a\nb\r\nc", b"a\nb\r\nc", "{}");
    assert_eq!(d.left.line_terminator, Some(LineTerminator::Mixed));
    assert_eq!(
        codes(&d),
        [
            ("mixed-line-endings", Some(Side::Left)),
            ("no-newline-at-eof", Some(Side::Left)),
            ("mixed-line-endings", Some(Side::Right)),
            ("no-newline-at-eof", Some(Side::Right)),
        ]
    );
    let d = doc(b"a", b"a", "{}");
    assert_eq!(d.left.line_terminator, Some(LineTerminator::None));
}

#[test]
fn missing_final_newline_is_a_change_only_without_whitespace_ignore() {
    let d = doc(b"a\nb\n", b"a\nb", "{}");
    assert_eq!(d.status, Status::Different);
    assert_eq!(d.hunks[0].ops.last().unwrap().op, OpKind::Replace);
    let d = doc(
        b"a\nb\n",
        b"a\nb",
        r#"{"ignore": {"whitespace": "change"}}"#,
    );
    assert_eq!(d.status, Status::Identical);
}

#[test]
fn whitespace_and_case_ignores() {
    let d = doc(
        b"a  b\n",
        b"a b \n",
        r#"{"ignore": {"whitespace": "change"}}"#,
    );
    assert_eq!(d.status, Status::Identical);
    let d = doc(b"ab\n", b"a b\n", r#"{"ignore": {"whitespace": "change"}}"#);
    assert_eq!(d.status, Status::Different);
    let d = doc(b"ab\n", b" a b\n", r#"{"ignore": {"whitespace": "all"}}"#);
    assert_eq!(d.status, Status::Identical);
    let d = doc(
        "Straße\n".as_bytes(),
        "STRASSE\n".as_bytes(),
        r#"{"ignore": {"case": true}}"#,
    );
    assert_eq!(d.status, Status::Different, "simple folding keeps ß");
    let d = doc(
        "ΣΑΣ\n".as_bytes(),
        "σας\n".as_bytes(),
        r#"{"ignore": {"case": true}}"#,
    );
    assert_eq!(d.status, Status::Identical);
}

#[test]
fn invalid_utf8_is_an_error_document_with_the_text_adapter() {
    let text = r#"{"adapter": "text"}"#;
    let d = doc(b"\xEF\xBB\xBFok\n\xff\n", b"ok", text);
    assert_eq!(d.status, Status::Error);
    assert!(d.stats.is_none());
    assert!(d.hunks.is_empty());
    assert_eq!(d.left.encoding, None);
    assert_eq!(d.left.line_count, None);
    assert!(d.left.bom);
    assert_eq!(d.right.line_count, Some(1));
    assert_eq!(
        codes(&d),
        [
            ("invalid-utf8", Some(Side::Left)),
            ("no-newline-at-eof", Some(Side::Right)),
        ]
    );
    // Overlong encodings and surrogates are invalid too.
    assert_eq!(doc(b"\xC0\xAF", b"", text).status, Status::Error);
    assert_eq!(doc(b"\xED\xA0\x80", b"", text).status, Status::Error);
    // UTF-16 with a BOM fails the UTF-8 check.
    assert_eq!(doc(b"\xFF\xFEa\x00", b"", text).status, Status::Error);
    // Valid UTF-8 with NUL bytes is compared as text when text is forced.
    let d = doc(b"a\0b\n", b"a\0c\n", text);
    assert_eq!((d.kind, d.status), (Kind::Text, Status::Different));
}

#[test]
fn inputs_that_are_not_utf8_text_are_compared_as_bytes() {
    let d = doc(b"\xEF\xBB\xBFok\n\xff\n", b"ok", "{}");
    assert_eq!((d.kind, d.status), (Kind::Binary, Status::Different));
    assert!(d.hunks.is_empty() && d.moves.is_empty());
    assert_eq!(codes(&d), [("binary-fallback", None)]);
    // No line meta and no per-side text warnings in a binary document.
    assert_eq!(d.left.line_count, None);
    assert_eq!(d.right.line_count, None);
    assert_eq!(d.right.final_newline, None);
    assert_eq!(d.left.encoding, None);
    assert_eq!(d.right.encoding.as_deref(), Some("utf-8"));
    assert!(d.left.bom);
    assert_eq!(
        d.left.media_type.as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(d.right.media_type.as_deref(), Some("text/plain"));
    let stats = d.stats.as_ref().unwrap();
    let bytes = stats.bytes.as_ref().unwrap();
    assert_eq!((bytes.left, bytes.right), (8, 2));
    assert_eq!(bytes.unchanged + bytes.removed, bytes.left);
    assert_eq!(bytes.unchanged + bytes.added, bytes.right);
    assert_eq!(stats.change_count, d.bytes.len() as u64);
    assert_eq!(stats.hunk_count, 0);
    // UTF-16 text is text, but not text the line diff reads.
    let d = doc(b"\xFF\xFEa\x00\n\x00", b"a\n", "{}");
    assert_eq!(d.kind, Kind::Binary);
    assert_eq!(d.left.encoding.as_deref(), Some("utf-16le"));
    assert_eq!(d.left.media_type.as_deref(), Some("text/plain"));
    assert_eq!(codes(&d), [("binary-fallback", None)]);
    // NUL bytes make an input binary.
    let d = doc(b"a\0b\n", b"a\0c\n", "{}");
    assert_eq!(d.kind, Kind::Binary);
    assert!(
        codes(&d).is_empty(),
        "both sides binary: no fallback warning"
    );
    assert_eq!(
        d.bytes,
        [ByteRange {
            left_offset: 2,
            left_length: 1,
            right_offset: 2,
            right_length: 1,
            op: OpKind::Replace
        }]
    );
    assert_eq!(
        d.stats.unwrap().similarity,
        Similarity::from_basis_points(7500)
    );
}

#[test]
fn binary_documents() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0";
    // Identical binaries.
    let d = doc(PNG, PNG, "{}");
    assert_eq!((d.kind, d.status), (Kind::Binary, Status::Identical));
    assert!(d.bytes.is_empty());
    assert_eq!(d.stats.as_ref().unwrap().similarity, Similarity::ONE);
    assert_eq!(d.left.media_type.as_deref(), Some("image/png"));
    assert_eq!(d.provenance.algorithm, "binary.bytes@1");
    assert_eq!(d.provenance.adapter.id, "binary");
    // Empty versus binary: everything inserted, no fallback warning.
    let d = doc(b"", PNG, "{}");
    assert_eq!(d.kind, Kind::Binary);
    assert!(codes(&d).is_empty());
    assert_eq!(d.bytes.len(), 1);
    assert_eq!(d.bytes[0].op, OpKind::Insert);
    assert_eq!(
        d.stats.as_ref().unwrap().similarity,
        Similarity::from_basis_points(0)
    );
    // Both empty under the binary adapter.
    let d = doc(b"", b"", r#"{"adapter": "binary"}"#);
    assert_eq!((d.kind, d.status), (Kind::Binary, Status::Identical));
    assert_eq!(d.stats.as_ref().unwrap().similarity, Similarity::ONE);
    // Text forced through the binary adapter.
    let d = doc(LEFT, RIGHT, r#"{"adapter": "binary", "detectMoves": true}"#);
    assert_eq!((d.kind, d.status), (Kind::Binary, Status::Different));
    assert!(codes(&d).is_empty(), "no text warnings and no fallback");
    assert!(!d.provenance.extra.contains_key("moves"));
    assert_eq!(d.left.encoding.as_deref(), Some("utf-8"));
    // Option errors still apply.
    let d = doc(PNG, b"", r#"{"ignore": {"regex": ["("]}}"#);
    assert_eq!((d.kind, d.status), (Kind::Binary, Status::Error));
    assert!(d.stats.is_none() && d.bytes.is_empty());
    // A binary document's JSON has the byte ranges and the byte stats.
    let json = diff(b"\x00\x01\x02", b"\x00\x09\x02", "{}").unwrap();
    assert!(json.contains(r#""bytes":[{"leftLength":1,"leftOffset":1,"op":"replace","rightLength":1,"rightOffset":1}]"#), "{json}");
    assert!(
        json.contains(r#""bytes":{"added":1,"left":3,"removed":1,"right":3,"unchanged":2}"#),
        "{json}"
    );
}

#[test]
fn identical_inputs_with_different_names_get_their_own_media_types() {
    let options = parse_options("{}").unwrap();
    let d = diff_document(
        Input::named(b"a,b\n", "x.txt"),
        Input::named(b"a,b\n", "x.csv"),
        &options,
    )
    .unwrap();
    assert_eq!(d.left.media_type.as_deref(), Some("text/plain"));
    assert_eq!(d.right.media_type.as_deref(), Some("text/csv"));
    assert_eq!(d.status, Status::Identical);
}

fn kinds(d: &DiffDocument) -> Vec<Vec<OpKind>> {
    d.hunks
        .iter()
        .map(|h| h.ops.iter().map(|o| o.op).collect())
        .collect()
}

const BLANK_LEFT: &[u8] = b"one\n\ntwo\nthree\nfour\nfive\nsix\n\n\nseven\n";
const BLANK_RIGHT: &[u8] = b"one\ntwo\nthree\nfour\nfive\nsix\n  \nseven\n\n";

#[test]
fn blank_only_changes_are_ignored() {
    let plain = doc(BLANK_LEFT, BLANK_RIGHT, r#"{"context": 1}"#);
    assert_eq!(plain.status, Status::Different);
    let d = doc(
        BLANK_LEFT,
        BLANK_RIGHT,
        r#"{"context": 1, "ignore": {"blankLines": true}}"#,
    );
    // Every change is blank on both sides, so the inputs are identical.
    assert_eq!(d.status, Status::Identical);
    assert!(d.hunks.is_empty());
    let stats = d.stats.unwrap();
    assert_eq!((stats.change_count, stats.hunk_count), (0, 0));
    assert_eq!((stats.lines_added, stats.lines_removed), (0, 0));
    assert_eq!((stats.lines_left, stats.lines_right), (10, 9));
    assert_eq!(stats.lines_unchanged, 7);
    // Ignored lines leave the similarity denominator: 2 * 7 / (7 + 7).
    assert_eq!(stats.similarity, Similarity::ONE);
    assert!(d.options.ignore.blank_lines);
}

#[test]
fn ignored_ops_appear_next_to_real_changes() {
    let right = b"one\ntwo\nthree\nFOUR\nfive\nsix\n  \nseven\n\n";
    let d = doc(
        BLANK_LEFT,
        right,
        r#"{"context": 3, "ignore": {"blankLines": true}}"#,
    );
    assert_eq!(d.status, Status::Different);
    // Three equal lines of context on each side of the change; the ignored ops
    // in between are kept whole and do not count. The blank line appended at
    // the end lies beyond the context.
    assert_eq!(
        kinds(&d),
        [[
            OpKind::Equal,
            OpKind::Ignored,
            OpKind::Equal,
            OpKind::Replace,
            OpKind::Equal,
            OpKind::Ignored,
            OpKind::Equal
        ]]
    );
    let ops = &d.hunks[0].ops;
    assert_eq!(ops[1].left_range, Range { start: 2, count: 1 });
    assert_eq!(ops[1].right_range, Range { start: 2, count: 0 });
    assert!(ops[1].inline.is_empty());
    // Hunk ranges still tile the ops.
    assert_eq!(
        d.hunks[0].left,
        Range {
            start: 1,
            count: 10
        }
    );
    assert_eq!(d.hunks[0].right, Range { start: 1, count: 8 });
    let stats = d.stats.as_ref().unwrap();
    assert_eq!(
        (stats.change_count, stats.lines_added, stats.lines_removed),
        (1, 1, 1)
    );
    assert_eq!(stats.lines_unchanged, 6);
    // 2 * 6 / (7 + 7) = 0.857142... -> 0.8571
    assert_eq!(stats.similarity.basis_points(), 8571);
    let json = model::to_canonical_json(&d).unwrap();
    assert!(json.contains(r#""op":"ignored""#));
}

#[test]
fn mixed_blank_and_text_changes_stay_changes() {
    let d = doc(
        b"a\nb\n",
        b"a\n\nB\n",
        r#"{"ignore": {"blankLines": true}}"#,
    );
    assert_eq!(d.status, Status::Different);
    assert_eq!(kinds(&d), [[OpKind::Equal, OpKind::Replace]]);
    // A whitespace-only line is blank; a terminator change of a blank line is too.
    let d = doc(
        b"x\n\t\n",
        b"x\n\r\n",
        r#"{"ignore": {"blankLines": true}}"#,
    );
    assert_eq!(d.status, Status::Identical);
    // U+00A0 is not whitespace, so that line is not blank.
    let d = doc(
        "x\n".as_bytes(),
        "x\n\u{a0}\n".as_bytes(),
        r#"{"ignore": {"blankLines": true}}"#,
    );
    assert_eq!(d.status, Status::Different);
}

#[test]
fn ignored_lines_are_never_moved() {
    let left = "fn alpha_function(first, second)\nfn beta_function(first, second)\nkeep 1\nkeep 2\nkeep 3\n\n\n\nkeep 4\n";
    let right = "keep 1\nkeep 2\nkeep 3\nkeep 4\nfn alpha_function(first, second)\nfn beta_function(first, second)\n";
    let d = doc(
        left.as_bytes(),
        right.as_bytes(),
        r#"{"detectMoves": true, "ignore": {"blankLines": true}}"#,
    );
    assert_eq!(d.moves.len(), 1);
    assert_eq!(d.moves[0].from.range, Range { start: 1, count: 2 });
    let ignored: Vec<&Op> = d
        .hunks
        .iter()
        .flat_map(|h| &h.ops)
        .filter(|o| o.op == OpKind::Ignored)
        .collect();
    assert_eq!(ignored.len(), 1);
    assert_eq!(ignored[0].left_range, Range { start: 6, count: 3 });
}

#[test]
fn regex_masks_compare_lines_without_the_matches() {
    let left = b"2026-01-01T10:00:00Z start job 7\n2026-01-01T10:00:05Z done job 7\n";
    let right = b"2026-02-03T11:30:00Z start job 7\n2026-02-03T11:30:09Z failed job 7\n";
    let opts = r#"{"ignore": {"regex": ["^\\d{4}-\\d{2}-\\d{2}T[0-9:]{8}Z"]}, "context": 0}"#;
    let d = doc(left, right, opts);
    assert_eq!(d.status, Status::Different);
    assert_eq!(kinds(&d), [[OpKind::Replace]]);
    assert_eq!(d.hunks[0].ops[0].left_range, Range { start: 2, count: 1 });
    assert_eq!(d.options.ignore.regex, [r"^\d{4}-\d{2}-\d{2}T[0-9:]{8}Z"]);
    // Masking everything that differs gives `identical`.
    let all = r#"{"ignore": {"regex": ["^\\S+", "done|failed"]}}"#;
    assert_eq!(doc(left, right, all).status, Status::Identical);
    // A line that lacks the masked text entirely still differs.
    let d = doc(
        b"id 42 ok\n",
        b"id  ok\n",
        r#"{"ignore": {"regex": ["\\d+"]}}"#,
    );
    assert_eq!(d.status, Status::Different);
}

#[test]
fn bad_regex_patterns_give_error_documents() {
    for (options, code) in [
        (r#"{"ignore": {"regex": ["(unclosed"]}}"#, "invalid-regex"),
        (r#"{"ignore": {"regex": ["a(?=b)"]}}"#, "invalid-regex"),
        (r#"{"ignore": {"regex": ["\\w{100}"]}}"#, "limit-exceeded"),
    ] {
        let d = doc(b"a\n", b"b\n", options);
        assert_eq!(d.status, Status::Error, "{options}");
        assert_eq!(codes(&d), [(code, None)], "{options}");
        assert!(d.stats.is_none() && d.hunks.is_empty());
        assert_eq!(d.left.line_count, Some(1));
    }
    let many: Vec<String> = (0..=key::MAX_REGEX_PATTERNS)
        .map(|i| format!("p{i}"))
        .collect();
    let options = json!({"ignore": {"regex": many}}).to_string();
    assert_eq!(
        codes(&doc(b"a\n", b"b\n", &options)),
        [("limit-exceeded", None)]
    );
    let both = r#"{"ignore": {"regex": ["(", "\\w{100}"]}}"#;
    assert_eq!(
        codes(&doc(b"a\n", b"b\n", both)),
        [("invalid-regex", None), ("limit-exceeded", None)]
    );
    // Pattern errors are reported even when an input does not decode.
    let d = doc(
        b"\xff\n",
        b"b\n",
        r#"{"adapter": "text", "ignore": {"regex": ["("]}}"#,
    );
    assert_eq!(
        codes(&d),
        [("invalid-utf8", Some(Side::Left)), ("invalid-regex", None)]
    );
}

#[test]
fn invalid_options_are_rejected_without_a_document() {
    assert!(matches!(
        diff(b"a", b"b", r#"{"bogus": true}"#),
        Err(Error::InvalidOptions(_))
    ));
}

#[test]
fn typed_api_rejects_invalid_options() {
    let bad = [
        Options {
            granularity: Granularity::Unknown,
            ..Options::default()
        },
        Options {
            context: model::MAX_CONTEXT + 1,
            ..Options::default()
        },
    ];
    for options in &bad {
        assert!(matches!(
            diff_document(Input::new(b"a\n"), Input::new(b"b\n"), options),
            Err(Error::InvalidOptions(_))
        ));
        assert!(matches!(
            diff_document_with(
                Input::new(b"a\n"),
                Input::new(b"b\n"),
                options,
                &Settings::default()
            ),
            Err(Error::InvalidOptions(_))
        ));
    }
    let ok = Options {
        context: model::MAX_CONTEXT,
        ..Options::default()
    };
    assert!(diff_document(Input::new(b"a\n"), Input::new(b"b\n"), &ok).is_ok());
}

#[test]
fn context_zero_and_large() {
    let left = b"1\n2\n3\n4\n5\n6\n7\n8\n";
    let right = b"1\nX\n3\n4\n5\n6\nY\n8\n";
    let d = doc(left, right, r#"{"context": 0}"#);
    assert_eq!(d.hunks.len(), 2);
    assert!(d.hunks.iter().all(|h| h.ops.len() == 1));
    let d = doc(left, right, r#"{"context": 2}"#);
    assert_eq!(d.hunks.len(), 1, "gap of 4 lines <= 2*2");
    let d = doc(left, right, r#"{"context": 2147483647}"#);
    assert_eq!(d.hunks.len(), 1);
    assert_eq!(d.hunks[0].left, Range { start: 1, count: 8 });
}

#[test]
fn hunks_tile_their_ranges() {
    let left = b"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
    let right = b"a\nB\nc\nd\nx\ny\ne\nf\nh\ni\nj\nk\n";
    for algorithm in ["histogram", "myers", "patience"] {
        let d = doc(left, right, &format!(r#"{{"algorithm": "{algorithm}"}}"#));
        assert_eq!(d.provenance.algorithm, format!("text.{algorithm}@1"));
        for hunk in &d.hunks {
            let (mut l, mut r) = (hunk.left.start, hunk.right.start);
            for op in &hunk.ops {
                assert_eq!((op.left_range.start, op.right_range.start), (l, r));
                l += op.left_range.count;
                r += op.right_range.count;
            }
            assert_eq!(l - hunk.left.start, hunk.left.count);
            assert_eq!(r - hunk.right.start, hunk.right.count);
        }
        let stats = d.stats.unwrap();
        assert_eq!(stats.lines_unchanged + stats.lines_removed, 10);
        assert_eq!(stats.lines_unchanged + stats.lines_added, 12);
    }
}

#[test]
fn similarity_rounding() {
    assert_eq!(similarity(4, 5, 6).basis_points(), 7273);
    assert_eq!(similarity(1, 2, 2).basis_points(), 5000);
    // 2/3 = 0.66666... -> 0.6667
    assert_eq!(similarity(1, 1, 2).basis_points(), 6667);
    assert_eq!(similarity(0, 0, 0).basis_points(), 10_000);
}

#[test]
fn word_granularity_spans_use_absolute_lines() {
    let d = doc(
        b"keep\nthe red fox\n",
        b"keep\nthe blue fox\n",
        r#"{"granularity": "word"}"#,
    );
    let op = &d.hunks[0].ops[1];
    assert_eq!(op.op, OpKind::Replace);
    assert_eq!(
        op.inline,
        [
            Span {
                side: Side::Left,
                line: 2,
                start: 4,
                end: 7,
                op: OpKind::Delete
            },
            Span {
                side: Side::Right,
                line: 2,
                start: 4,
                end: 8,
                op: OpKind::Insert
            },
        ]
    );
    // Line granularity never has spans.
    let d = doc(b"keep\nthe red fox\n", b"keep\nthe blue fox\n", "{}");
    assert!(d.hunks[0].ops.iter().all(|o| o.inline.is_empty()));
}

#[test]
fn utf16_positions() {
    let d = doc(
        "😀 a\n".as_bytes(),
        "😀 b\n".as_bytes(),
        r#"{"granularity": "char", "positionEncoding": "utf-16"}"#,
    );
    assert_eq!(d.position_encoding, PositionEncoding::Utf16);
    let spans = &d.hunks[0].ops[0].inline;
    assert_eq!((spans[0].start, spans[0].end), (3, 4));
}

#[test]
fn refine_limit_warning() {
    let left = "line\n".repeat(101);
    let right = "LINE\n".repeat(101);
    let d = doc(
        left.as_bytes(),
        right.as_bytes(),
        r#"{"granularity": "smart"}"#,
    );
    assert_eq!(d.status, Status::Different);
    assert_eq!(codes(&d), [("refine-skipped-limit", None)]);
    assert!(d.hunks[0].ops[0].inline.is_empty());
}

#[test]
fn large_rearrangement_is_approximate() {
    // 600 lines over a 20-line vocabulary, reversed: far more than 500 changed
    // lines whose content occurs on the other side.
    let lines: Vec<String> = (0..600).map(|i| format!("l{}\n", (i * 7) % 20)).collect();
    let left = lines.concat();
    let right: String = lines.iter().rev().cloned().collect();
    for alg in ["histogram", "myers", "patience"] {
        let d = doc(
            left.as_bytes(),
            right.as_bytes(),
            &format!(r#"{{"algorithm": "{alg}"}}"#),
        );
        assert_eq!(d.status, Status::Different, "{alg}");
        assert_eq!(codes(&d), [("approximate", None)], "{alg}");
    }
    // A small edit of the same file is exact.
    let edited = left.replacen("l7\n", "x\n", 1);
    let d = doc(left.as_bytes(), edited.as_bytes(), "{}");
    assert!(codes(&d).is_empty());
    // A pure insertion of 600 lines that all occur on the other side runs no
    // search, so it is exact; so is the deletion back.
    let grown = [
        lines[..10].concat(),
        "l7\n".repeat(600),
        lines[10..].concat(),
    ]
    .concat();
    for alg in ["histogram", "myers", "patience"] {
        let options = format!(r#"{{"algorithm": "{alg}"}}"#);
        for (l, r) in [(&left, &grown), (&grown, &left)] {
            let d = doc(l.as_bytes(), r.as_bytes(), &options);
            assert_eq!(d.status, Status::Different, "{alg}");
            assert!(codes(&d).is_empty(), "{alg}: {:?}", codes(&d));
        }
    }
}

#[test]
fn input_names_keep_the_final_component() {
    assert_eq!(Input::named(b"", "a/b/c.txt").name(), Some("c.txt"));
    assert_eq!(Input::named(b"", r"C:\x\y.txt").name(), Some("y.txt"));
    assert_eq!(Input::named(b"", "dir/").name(), None);
    assert_eq!(Input::new(b"").name(), None);
}

const MOVE_LEFT: &str = "header line\nfn moved_function_one(argument_value)\nfn moved_function_two(argument_value)\nthe old wording here\nstable one\nstable two\nstable three\nstable four\n";
const MOVE_RIGHT: &str = "header line\nthe new wording here\nstable one\nstable two\nstable three\nstable four\nfn moved_function_one(argument_value)\nfn moved_function_two(argument_value)\n";

#[test]
fn moves_reference_ops_and_leave_hunks_unchanged() {
    let with = doc(
        MOVE_LEFT.as_bytes(),
        MOVE_RIGHT.as_bytes(),
        r#"{"detectMoves": true, "context": 1}"#,
    );
    let without = doc(
        MOVE_LEFT.as_bytes(),
        MOVE_RIGHT.as_bytes(),
        r#"{"context": 1}"#,
    );
    assert_eq!(with.status, Status::Different);
    assert!(codes(&with).is_empty());
    assert_eq!(with.hunks, without.hunks);
    assert_eq!(with.stats, without.stats);
    assert!(without.moves.is_empty());
    assert_eq!(with.moves.len(), 1);
    let json: Value = serde_json::from_str(&model::to_canonical_json(&with).unwrap()).unwrap();
    // Left 2-3 sit in the replace op of h1; right 7-8 are the insert of h2.
    assert_eq!(
        json["moves"],
        json!([{
            "id": "m1",
            "kind": "pure",
            "from": { "side": "left", "range": { "start": 2, "count": 2 } },
            "to": { "side": "right", "range": { "start": 7, "count": 2 } },
            "similarity": 1,
            "fromOps": [{ "hunk": "h1", "op": 1 }],
            "toOps": [{ "hunk": "h2", "op": 1 }],
            "inline": []
        }])
    );
    assert_eq!(with.hunks[0].ops[1].op, OpKind::Replace);
    assert_eq!(with.hunks[1].ops[1].op, OpKind::Insert);
    assert_eq!(
        json["provenance"]["moves"],
        Value::String(moves::MOVES_ID.to_owned())
    );
    // Op references follow the hunk grouping: one hunk with a large context.
    let wide = doc(
        MOVE_LEFT.as_bytes(),
        MOVE_RIGHT.as_bytes(),
        r#"{"detectMoves": true, "context": 10}"#,
    );
    assert_eq!(
        wide.moves[0].from_ops,
        [OpRef {
            hunk: "h1".to_owned(),
            op: 1
        }]
    );
    assert_eq!(
        wide.moves[0].to_ops,
        [OpRef {
            hunk: "h1".to_owned(),
            op: 3
        }]
    );
}

#[test]
fn moved_lines_get_no_inline_spans() {
    let opts = |moves: bool| format!(r#"{{"detectMoves": {moves}, "granularity": "word"}}"#);
    let with = doc(MOVE_LEFT.as_bytes(), MOVE_RIGHT.as_bytes(), &opts(true));
    let without = doc(MOVE_LEFT.as_bytes(), MOVE_RIGHT.as_bytes(), &opts(false));
    let spans = |d: &DiffDocument| -> Vec<(Side, u64)> {
        d.hunks
            .iter()
            .flat_map(|h| &h.ops)
            .flat_map(|o| &o.inline)
            .map(|s| (s.side, s.line))
            .collect()
    };
    let moved = spans(&with);
    assert!(!moved.is_empty());
    // Only the reworded line pair (left 4, right 2) is refined.
    assert!(
        moved
            .iter()
            .all(|s| *s == (Side::Left, 4) || *s == (Side::Right, 2)),
        "{moved:?}"
    );
    assert!(
        spans(&without)
            .iter()
            .any(|s| s.0 == Side::Left && (s.1 == 2 || s.1 == 3))
    );
    // A pure move carries no spans of its own.
    assert_eq!(with.moves[0].kind, MoveKind::Pure);
    assert!(with.moves[0].inline.is_empty());
}

const EDITED_LEFT: &str = "fn intro() {}\n\nfn checksum(data) {\n  let acc = 0;\n  for byte in data {\n    acc = acc * 31 + byte;\n  }\n  return acc;\n}\n\nfn middle() {}\nfn more() {}\nfn other() {}\n";
const EDITED_RIGHT: &str = "fn intro() {}\n\nfn middle() {}\nfn more() {}\nfn other() {}\n\nfn checksum(data) {\n    let hash = 0;\n    for byte in data {\n      hash = hash * 31 + byte;\n    }\n    return hash;\n}\n";

/// The text of each move span, as (side, line, text).
fn move_span_texts(d: &DiffDocument, left: &str, right: &str) -> Vec<(Side, u64, String)> {
    d.moves
        .iter()
        .flat_map(|m| &m.inline)
        .map(|s| {
            let text = if s.side == Side::Left { left } else { right };
            let line = text
                .lines()
                .nth(usize::try_from(s.line).unwrap() - 1)
                .unwrap();
            let range = usize::try_from(s.start).unwrap()..usize::try_from(s.end).unwrap();
            (s.side, s.line, line[range].to_owned())
        })
        .collect()
}

#[test]
fn edited_moves_carry_inline_spans() {
    let d = doc(
        EDITED_LEFT.as_bytes(),
        EDITED_RIGHT.as_bytes(),
        r#"{"detectMoves": true, "granularity": "smart"}"#,
    );
    assert_eq!(d.moves.len(), 1, "{:?}", d.moves);
    let m = &d.moves[0];
    assert_eq!(m.kind, MoveKind::Edited);
    // Absolute line numbers, left spans first; the new indentation of the
    // moved lines gets no span.
    let texts = move_span_texts(&d, EDITED_LEFT, EDITED_RIGHT);
    let acc = |line| (Side::Left, line, "acc".to_owned());
    let hash = |line| (Side::Right, line, "hash".to_owned());
    assert_eq!(
        texts,
        [
            acc(4),
            acc(6),
            acc(6),
            acc(8),
            hash(8),
            hash(10),
            hash(10),
            hash(12)
        ]
    );
    assert!(m.inline.iter().all(|s| match s.side {
        Side::Left => s.op == OpKind::Delete,
        _ => s.op == OpKind::Insert,
    }));
    // The ops keep no spans for moved lines, and nothing else changed.
    assert!(
        d.hunks
            .iter()
            .flat_map(|h| &h.ops)
            .all(|o| o.inline.is_empty())
    );
    assert!(codes(&d).is_empty());

    // Granularity `line`: the same move, no spans.
    let plain = doc(
        EDITED_LEFT.as_bytes(),
        EDITED_RIGHT.as_bytes(),
        r#"{"detectMoves": true}"#,
    );
    assert_eq!(plain.moves.len(), 1);
    assert_eq!(plain.moves[0].from, m.from);
    assert!(plain.moves[0].inline.is_empty());

    // ASCII text: UTF-16 offsets equal UTF-8 ones, and `word` gives the
    // spans of `smart` here (every changed word is shorter than 4).
    let utf16 = doc(
        EDITED_LEFT.as_bytes(),
        EDITED_RIGHT.as_bytes(),
        r#"{"detectMoves": true, "granularity": "word", "positionEncoding": "utf-16"}"#,
    );
    assert_eq!(utf16.moves[0].inline, m.inline);
}

#[test]
fn edited_moves_over_the_refine_limit_get_no_spans() {
    let block: Vec<String> = (0..=refine::REFINE_MAX_LINES)
        .map(|i| format!("let value_number_{i} = compute_the_thing({i});"))
        .collect();
    let mut edited = block.clone();
    edited[50] = "let value_number_50 = compute_other_thing(50);".to_owned();
    // More stable lines than moved ones, so the line diff keeps them in place.
    let stable: Vec<String> = (0..150).map(|i| format!("stable line {i}")).collect();
    let stable = stable.join("\n");
    let left = format!("head\n{}\n{stable}\n", block.join("\n"));
    let right = format!("head\n{stable}\n{}\n", edited.join("\n"));
    let d = doc(
        left.as_bytes(),
        right.as_bytes(),
        r#"{"detectMoves": true, "granularity": "smart"}"#,
    );
    assert_eq!(d.moves.len(), 1, "{:?}", d.moves);
    assert_eq!(d.moves[0].kind, MoveKind::Edited);
    assert!(d.moves[0].from.range.count > refine::REFINE_MAX_LINES as u64);
    assert!(d.moves[0].inline.is_empty());
    assert_eq!(codes(&d), [("refine-skipped-limit", None)]);
}

#[test]
fn move_detection_on_identical_and_huge_inputs() {
    let d = doc(LEFT, LEFT, r#"{"detectMoves": true}"#);
    assert_eq!(d.status, Status::Identical);
    assert!(d.moves.is_empty());
    // More deleted lines than the candidate guard allows.
    let big: String = (0..=moves_limit()).fold(String::new(), |mut s, i| {
        let _ = writeln!(s, "line {i}");
        s
    });
    let d = doc(big.as_bytes(), b"other\n", r#"{"detectMoves": true}"#);
    assert_eq!(d.status, Status::Different);
    assert!(d.moves.is_empty());
    assert_eq!(codes(&d), [("moves-skipped-limit", None)]);
    assert!(!d.hunks.is_empty());
}

fn moves_limit() -> usize {
    moves::MOVE_MAX_CANDIDATE_LINES
}

/// Without change ops, move detection finds nothing and reports no limit,
/// so the engine skips it (and the per-line contents it needs).
#[test]
fn move_detection_without_change_ops_is_empty() {
    use diffyman_algo::ops::{LineOp, OpKind as AlgoOpKind};
    let lines: Vec<String> = (0..200)
        .map(|i| format!("    let value_{i} = compute({i});"))
        .collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let op = |kind, ls, ll, rs, rl| LineOp {
        kind,
        left_start: ls,
        left_len: ll,
        right_start: rs,
        right_len: rl,
    };
    for ops in [
        vec![op(AlgoOpKind::Equal, 0, 200, 0, 200)],
        vec![
            op(AlgoOpKind::Equal, 0, 100, 0, 100),
            op(AlgoOpKind::Ignored, 100, 50, 100, 0),
            op(AlgoOpKind::Equal, 150, 50, 100, 50),
        ],
    ] {
        let right = if ops.len() == 1 {
            &lines[..]
        } else {
            &lines[..150]
        };
        let keys = KeyBuilder::new(&KeyOptions::default()).unwrap();
        let found = moves::detect_moves(&lines, right, &ops, &keys);
        assert!(found.moves.is_empty());
        assert!(!found.skipped_limit);
    }
}

/// Byte-identical inputs are decoded once; the right side still gets its
/// own name and its own copy of every per-side warning.
#[test]
fn identical_bytes_report_both_sides() {
    let text = b"\xEF\xBB\xBFa\r\nb\nc";
    let d = doc(text, text, "{}");
    assert_eq!(d.status, Status::Identical);
    assert_eq!(d.left.name.as_deref(), Some("left.txt"));
    assert_eq!(d.right.name.as_deref(), Some("right.txt"));
    let mut right_as_left = d.right.clone();
    right_as_left.name.clone_from(&d.left.name);
    assert_eq!(right_as_left, d.left);
    assert_eq!(d.left.line_count, Some(3));
    let mut want = Vec::new();
    for side in [Side::Left, Side::Right] {
        for code in ["bom-stripped", "mixed-line-endings", "no-newline-at-eof"] {
            want.push((code, Some(side)));
        }
    }
    let mut got = codes(&d);
    got.sort_unstable();
    want.sort_unstable();
    assert_eq!(got, want);
    let stats = d.stats.unwrap();
    assert_eq!((stats.lines_unchanged, stats.change_count), (3, 0));
}

// ---------------------------------------------------------------------------
// Kind `tree` (spec §19)
// ---------------------------------------------------------------------------

/// A zip of stored entries: `(name, data, general purpose flags)`.
fn stored_zip(entries: &[(&str, &[u8], u16)]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
            }
        }
        !crc
    }
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data, flags) in entries {
        let fields = |out: &mut Vec<u8>| {
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&[0, 0, 0, 0, 0x21, 0]);
            out.extend_from_slice(&crc32(data).to_le_bytes());
            let size = u32::try_from(data.len()).unwrap().to_le_bytes();
            out.extend_from_slice(&size);
            out.extend_from_slice(&size);
            out.extend_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
            out.extend_from_slice(&[0, 0]);
        };
        let offset = u32::try_from(out.len()).unwrap();
        out.extend_from_slice(b"PK\x03\x04");
        fields(&mut out);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(b"PK\x01\x02\x14\x00");
        fields(&mut central);
        central.extend_from_slice(&[0; 10]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let start = u32::try_from(out.len()).unwrap();
    let count = u16::try_from(entries.len()).unwrap().to_le_bytes();
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&count);
    out.extend_from_slice(&count);
    out.extend_from_slice(&u32::try_from(central.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

fn tree_doc(left: &[u8], right: &[u8], options: &str) -> DiffDocument {
    let options = parse_options(options).unwrap();
    diff_document(
        Input::named(left, "left.zip"),
        Input::named(right, "right.zip"),
        &options,
    )
    .unwrap()
}

fn entry<'d>(d: &'d DiffDocument, path: &str) -> &'d model::TreeEntry {
    d.tree
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .find(|e| e.path == path)
        .unwrap_or_else(|| panic!("no entry {path}"))
}

#[test]
fn archives_give_tree_documents_with_child_documents() {
    let left = stored_zip(&[
        ("docs/readme.txt", b"hello\nworld\n", 0),
        ("same.bin", b"\x00\x01\x02", 0),
        ("old-name.txt", b"moved content\n", 0),
        ("gone.txt", b"bye\n", 0),
    ]);
    let right = stored_zip(&[
        ("docs/readme.txt", b"hello\nthere\n", 0),
        ("same.bin", b"\x00\x01\x02", 0),
        ("new-name.txt", b"moved content\n", 0),
        ("added.txt", b"hi\n", 0),
    ]);
    let d = tree_doc(&left, &right, "{}");
    assert_eq!(d.kind, Kind::Tree);
    assert_eq!(d.status, Status::Different);
    assert_eq!(d.provenance.algorithm, "tree.entries@1");
    assert_eq!(d.provenance.adapter.id, "tree");
    assert_eq!(d.left.media_type.as_deref(), Some("application/zip"));
    assert_eq!(d.left.line_count, None);
    let paths: Vec<&str> = d
        .tree
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|e| e.path.as_str())
        .collect();
    assert_eq!(
        paths,
        [
            "added.txt",
            "docs",
            "docs/readme.txt",
            "gone.txt",
            "new-name.txt",
            "same.bin"
        ]
    );
    let readme = entry(&d, "docs/readme.txt");
    assert_eq!(readme.status, model::EntryStatus::Modified);
    let child = readme.child_document.as_ref().unwrap();
    assert_eq!(child.kind, Kind::Text);
    assert_eq!(child.left.name.as_deref(), Some("readme.txt"));
    assert_eq!(child.stats.as_ref().unwrap().lines_added, 1);
    assert!(child.id.starts_with("sha256:"));
    let moved = entry(&d, "new-name.txt");
    assert_eq!(moved.status, model::EntryStatus::Moved);
    assert_eq!(moved.from_path.as_deref(), Some("old-name.txt"));
    assert!(moved.child_document.is_none());
    assert_eq!(entry(&d, "same.bin").status, model::EntryStatus::Unchanged);
    assert_eq!(
        entry(&d, "docs").left.as_ref().unwrap().entry_type,
        model::EntryType::Directory
    );
    let stats = d.stats.as_ref().unwrap();
    let t = stats.tree.as_ref().unwrap();
    assert_eq!(
        (
            t.entries_left,
            t.entries_right,
            t.added,
            t.removed,
            t.modified,
            t.moved,
            t.unchanged
        ),
        (5, 5, 1, 1, 1, 1, 2)
    );
    assert_eq!(stats.change_count, 4);
    assert_eq!(stats.similarity, Similarity::from_basis_points(4000));
    // The canonical text never holds a child's provenance, and is stable.
    let json = diff_inputs(
        Input::named(&left, "left.zip"),
        Input::named(&right, "right.zip"),
        "{}",
    )
    .unwrap();
    assert_eq!(json.matches("\"provenance\"").count(), 1);
    let again = diff_inputs(
        Input::named(&left, "left.zip"),
        Input::named(&right, "right.zip"),
        "{}",
    )
    .unwrap();
    let strip = |s: &str| {
        let mut v: Value = serde_json::from_str(s).unwrap();
        v.as_object_mut().unwrap().remove("provenance");
        v
    };
    assert_eq!(strip(&json), strip(&again));
    // Without move detection the rename is a removal and an addition.
    let plain = tree_doc(&left, &right, r#"{"tree": {"detectMoves": false}}"#);
    assert_eq!(
        entry(&plain, "new-name.txt").status,
        model::EntryStatus::Added
    );
    assert_eq!(
        entry(&plain, "old-name.txt").status,
        model::EntryStatus::Removed
    );
}

#[test]
fn tree_adapter_selection() {
    let zip = stored_zip(&[("a.txt", b"a\n", 0)]);
    let same = tree_doc(&zip, &zip, "{}");
    assert_eq!((same.kind, same.status), (Kind::Tree, Status::Identical));
    assert_eq!(same.stats.as_ref().unwrap().similarity, Similarity::ONE);
    let bytes = tree_doc(
        &zip,
        &stored_zip(&[("a.txt", b"b\n", 0)]),
        r#"{"adapter": "binary"}"#,
    );
    assert_eq!(bytes.kind, Kind::Binary);
    // Under `auto`, an archive against a plain file is a byte diff.
    let mixed = tree_doc(&zip, b"plain text\n", "{}");
    assert_eq!(mixed.kind, Kind::Binary);
    // Under `tree`, inputs that are not archives are errors.
    let forced = tree_doc(b"plain text\n", &zip, r#"{"adapter": "tree"}"#);
    assert_eq!((forced.kind, forced.status), (Kind::Tree, Status::Error));
    assert_eq!(codes(&forced), [("invalid-archive", Some(Side::Left))]);
    assert!(forced.tree.is_none() && forced.stats.is_none());
    assert_eq!(forced.right.media_type.as_deref(), Some("application/zip"));
    // A broken archive under `auto` is an error too.
    let broken = tree_doc(&zip[..zip.len() - 4], &zip, "{}");
    assert_eq!(codes(&broken), [("invalid-archive", Some(Side::Left))]);
}

#[test]
fn tree_option_errors_apply_to_every_kind() {
    let d = doc(LEFT, LEFT, r#"{"tree": {"exclude": ["[z-a]"]}}"#);
    assert_eq!((d.kind, d.status), (Kind::Text, Status::Error));
    assert_eq!(codes(&d), [("invalid-glob", None)]);
    let many: Vec<String> = (0..65).map(|i| format!("\"p{i}\"")).collect();
    let d = doc(
        LEFT,
        LEFT,
        &format!(r#"{{"tree": {{"exclude": [{}]}}}}"#, many.join(",")),
    );
    assert_eq!(codes(&d), [("limit-exceeded", None)]);
    let zip = stored_zip(&[("a.log", b"1", 0), ("b.txt", b"2", 0)]);
    let excluded = tree_doc(
        &zip,
        &stored_zip(&[("b.txt", b"2", 0)]),
        r#"{"tree": {"exclude": ["*.log"]}}"#,
    );
    assert_eq!(excluded.status, Status::Identical);
}

#[test]
fn encrypted_entries_make_the_tree_partial() {
    let left = stored_zip(&[("secret", b"0123456789ab-data", 1), ("open.txt", b"x", 0)]);
    let right = stored_zip(&[("secret", b"0123456789ab-data", 1), ("open.txt", b"x", 0)]);
    let d = tree_doc(&left, &right, "{}");
    assert_eq!(d.status, Status::Partial);
    assert_eq!(
        codes(&d),
        [
            ("encrypted-entry", Some(Side::Left)),
            ("encrypted-entry", Some(Side::Right))
        ]
    );
    let secret = entry(&d, "secret");
    assert_eq!(
        secret.status,
        model::EntryStatus::Unchanged,
        "same stored bytes"
    );
    assert_eq!(secret.left.as_ref().unwrap().sha256, None);
    assert_eq!(secret.left.as_ref().unwrap().size, Some(17));
}

#[test]
fn child_budget_and_nesting_depth() {
    let left = stored_zip(&[("a.txt", b"one\n", 0), ("b.txt", b"two\n", 0)]);
    let right = stored_zip(&[("a.txt", b"ONE\n", 0), ("b.txt", b"TWO\n", 0)]);
    let full = tree_doc(&left, &right, "{}");
    let a_size = {
        let child = entry(&full, "a.txt").child_document.as_ref().unwrap();
        model::jcs::to_canonical_string(child.as_ref())
            .unwrap()
            .len()
    };
    // Room for exactly one child: the first in path order gets it.
    let one = tree_doc(
        &left,
        &right,
        &format!(r#"{{"tree": {{"childBudget": {a_size}}}}}"#),
    );
    assert!(entry(&one, "a.txt").child_document.is_some());
    assert!(entry(&one, "b.txt").child_document.is_none());
    assert_eq!(codes(&one), [("children-omitted", None)]);
    let none = tree_doc(&left, &right, r#"{"tree": {"childBudget": 0}}"#);
    assert!(entry(&none, "a.txt").child_document.is_none());
    assert!(none.warnings.is_empty(), "no warning when children are off");

    // Archives inside archives: trees down to maxDepth, then bytes.
    let outer_left = stored_zip(&[("inner.zip", &left, 0)]);
    let outer_right = stored_zip(&[("inner.zip", &right, 0)]);
    let nested = tree_doc(&outer_left, &outer_right, "{}");
    let child = entry(&nested, "inner.zip").child_document.as_ref().unwrap();
    assert_eq!(child.kind, Kind::Tree);
    let grandchild = child.tree.as_ref().unwrap().entries[0]
        .child_document
        .as_ref()
        .unwrap();
    assert_eq!(grandchild.kind, Kind::Text);
    let flat = tree_doc(&outer_left, &outer_right, r#"{"tree": {"maxDepth": 0}}"#);
    let child = entry(&flat, "inner.zip").child_document.as_ref().unwrap();
    assert_eq!(child.kind, Kind::Binary);
    assert_eq!(
        child
            .warnings
            .iter()
            .map(|w| w.code.as_str())
            .collect::<Vec<_>>(),
        ["depth-limit"]
    );
    // `adapter: "tree"` becomes `auto` in children.
    let forced = tree_doc(&outer_left, &outer_right, r#"{"adapter": "tree"}"#);
    let child = entry(&forced, "inner.zip").child_document.as_ref().unwrap();
    assert_eq!(child.options.adapter, Adapter::Auto);
    assert_eq!(forced.options.adapter, Adapter::Tree);
}

/// A nested tree sets its own size aside before its children use the
/// budget, so with a budget a little short of the whole nested child, the
/// child is embedded with fewer grandchildren instead of being computed in
/// full and then dropped.
#[test]
fn nested_trees_reserve_their_own_size() {
    let left = stored_zip(&[("a.txt", b"one\n", 0), ("b.txt", b"two\n", 0)]);
    let right = stored_zip(&[("a.txt", b"ONE\n", 0), ("b.txt", b"TWO\n", 0)]);
    let outer_left = stored_zip(&[("inner.zip", &left, 0)]);
    let outer_right = stored_zip(&[("inner.zip", &right, 0)]);
    let size = |d: &DiffDocument| {
        let child = entry(d, "inner.zip").child_document.as_ref().unwrap();
        model::jcs::to_canonical_string(child.as_ref())
            .unwrap()
            .len()
    };
    let full = tree_doc(&outer_left, &outer_right, "{}");
    let whole = size(&full);
    let tight = tree_doc(
        &outer_left,
        &outer_right,
        &format!(r#"{{"tree": {{"childBudget": {}}}}}"#, whole - 1),
    );
    assert!(tight.warnings.is_empty(), "the nested child fits");
    let child = entry(&tight, "inner.zip").child_document.as_ref().unwrap();
    let grandchildren: Vec<bool> = child
        .tree
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|e| e.child_document.is_some())
        .collect();
    assert_eq!(grandchildren, [true, false]);
    assert_eq!(
        child
            .warnings
            .iter()
            .map(|w| w.code.as_str())
            .collect::<Vec<_>>(),
        ["children-omitted"]
    );
    assert!(size(&tight) < whole);
    // A budget below the smallest possible child computes no child at all.
    let tiny = tree_doc(
        &outer_left,
        &outer_right,
        r#"{"tree": {"childBudget": 100}}"#,
    );
    assert!(entry(&tiny, "inner.zip").child_document.is_none());
    assert_eq!(codes(&tiny), [("children-omitted", None)]);
}

/// Child documents read archive content again, and that counts against the
/// per-side content budget (spec §19.6) like the listing does.
#[test]
fn child_contents_count_against_the_content_budget() {
    let left = stored_zip(&[("a.txt", &[b'a'; 100], 0)]);
    let right = stored_zip(&[("a.txt", &[b'b'; 100], 0)]);
    let options = parse_options("{}").unwrap();
    let run = |content_budget: u64| {
        let limits = diffyman_adapter_tree::Limits {
            content_budget,
            ..diffyman_adapter_tree::Limits::default()
        };
        let mut ctx = tree::context_with_limits(&options, limits);
        tree::archive_document(Input::new(&left), Input::new(&right), &options, &mut ctx)
    };
    // The listing reads 100 bytes per side, the child 100 more.
    let enough = run(200);
    assert!(entry(&enough, "a.txt").child_document.is_some());
    assert!(enough.warnings.is_empty());
    let short = run(199);
    assert!(entry(&short, "a.txt").child_document.is_none());
    assert_eq!(codes(&short), [("children-omitted", None)]);
    assert_eq!(short.status, Status::Different);
}

#[cfg(feature = "fs")]
#[test]
fn folders_compare_as_trees() {
    let dir = tempfile::tempdir().unwrap();
    let (left, right) = (dir.path().join("left"), dir.path().join("right"));
    for (root, text) in [(&left, "v1\n"), (&right, "v2\n")] {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.txt"), text).unwrap();
        std::fs::write(root.join("same.txt"), "same\n").unwrap();
    }
    let options = Options::default();
    let settings = Settings::default();
    let d = diff_paths(&left, &right, &options, &settings).unwrap();
    assert_eq!((d.kind, d.status), (Kind::Tree, Status::Different));
    assert_eq!(d.left.name.as_deref(), Some("left"));
    assert_eq!(d.left.media_type.as_deref(), Some("inode/directory"));
    assert_eq!(d.left.size, 8);
    assert_ne!(d.left.sha256, d.right.sha256);
    assert!(entry(&d, "src/main.txt").child_document.is_some());
    // The manifest digest depends only on the entries.
    let copy = dir.path().join("copy");
    std::fs::create_dir_all(copy.join("src")).unwrap();
    std::fs::write(copy.join("src/main.txt"), "v1\n").unwrap();
    std::fs::write(copy.join("same.txt"), "same\n").unwrap();
    let same = diff_paths(&left, &copy, &options, &settings).unwrap();
    assert_eq!(same.status, Status::Identical);
    assert_eq!(same.left.sha256, same.right.sha256);
    // A folder against a zip of the same files.
    let zip = dir.path().join("right.zip");
    std::fs::write(
        &zip,
        stored_zip(&[("src/main.txt", b"v1\n", 0), ("same.txt", b"same\n", 0)]),
    )
    .unwrap();
    let mixed = diff_paths(&left, &zip, &options, &settings).unwrap();
    assert_eq!((mixed.kind, mixed.status), (Kind::Tree, Status::Identical));
    assert_eq!(mixed.right.media_type.as_deref(), Some("application/zip"));
    // A folder against a plain file, and paths that do not exist.
    let plain = dir.path().join("plain.txt");
    std::fs::write(&plain, "text\n").unwrap();
    let bad = diff_paths(&left, &plain, &options, &settings).unwrap();
    assert_eq!(codes(&bad), [("invalid-archive", Some(Side::Right))]);
    let missing = dir.path().join("missing");
    assert!(matches!(
        diff_paths(&missing, &plain, &options, &settings),
        Err(Error::Io(_))
    ));
    assert!(matches!(
        diff_paths(&left, &missing, &options, &settings),
        Err(Error::Io(_))
    ));
    // Two files are an ordinary diff.
    let file_doc = diff_paths(&plain, &plain, &options, &settings).unwrap();
    assert_eq!(
        (file_doc.kind, file_doc.status),
        (Kind::Text, Status::Identical)
    );
    assert_eq!(file_doc.left.name.as_deref(), Some("plain.txt"));
}

/// The file paired with a folder is listed as an archive, which is at most
/// 4 GiB (spec §19.1, §19.6): a larger one is refused by its size before
/// anything is read, whatever read limit the caller gives.
#[cfg(all(feature = "fs", unix))]
#[test]
fn a_file_paired_with_a_folder_is_refused_over_the_archive_limit() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("a.txt"), "a\n").unwrap();
    // A sparse file: no data is written, and none may be read.
    let big = dir.path().join("big.zip");
    std::fs::File::create(&big)
        .unwrap()
        .set_len((1 << 32) + 1)
        .unwrap();
    let options = Options::default();
    let settings = Settings::default();
    for (left, right) in [(&folder, &big), (&big, &folder)] {
        for limit in [None, Some(u64::MAX)] {
            let started = std::time::Instant::now();
            let result = diff_paths_with_limit(left, right, &options, &settings, limit);
            let Err(Error::Io(message)) = result else {
                panic!("not refused: {result:?}");
            };
            assert!(message.starts_with("limit-exceeded: "), "{message}");
            assert!(
                message.ends_with(" is larger than 4294967296 bytes"),
                "{message}"
            );
            assert!(started.elapsed().as_secs() < 5, "read before refusing");
        }
        // A smaller limit still applies.
        let small = diff_paths_with_limit(left, right, &options, &settings, Some(10));
        let Err(Error::Io(message)) = small else {
            panic!("not refused: {small:?}");
        };
        assert!(message.ends_with(" is larger than 10 bytes"), "{message}");
    }
}

/// Every input the tree adapter reads as an archive starts like one (spec
/// §19.7), so a build without archive support refuses every pair a full
/// build compares as a tree. The check needs no decompression, so it also
/// matches gzip data that holds no tar.
#[test]
fn archives_start_like_archives() {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let mut archives = 0;
    for group in ["binary", "tree", "text"] {
        for case in std::fs::read_dir(corpus.join(group)).unwrap() {
            for file in std::fs::read_dir(case.unwrap().path()).unwrap() {
                let path = file.unwrap().path();
                if !path.is_file() {
                    continue;
                }
                let bytes = std::fs::read(&path).unwrap();
                if tree::archive_format(&bytes).is_some() {
                    archives += 1;
                    assert!(starts_like_archive(&bytes), "{}", path.display());
                }
            }
        }
    }
    assert!(archives > 20, "{archives}");
    assert!(starts_like_archive(&[0x1f, 0x8b, 8, 0]));
    assert!(tree::archive_format(&[0x1f, 0x8b, 8, 0]).is_none());
    assert!(!starts_like_archive(b"PK"));
    assert!(!starts_like_archive(b"plain text\n"));
    assert_eq!(FEATURES, ["archives", "fs"]);
    assert_eq!(Error::Unsupported(String::new()).code(), "unsupported");
}
