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
        "schemaVersion": "0.1.0",
        "kind": "text",
        "status": "different",
        "positionEncoding": "utf-8",
        "stats": {
            "linesLeft": 5, "linesRight": 6, "linesAdded": 2, "linesRemoved": 1,
            "linesUnchanged": 4, "changeCount": 2, "hunkCount": 2, "similarity": 0.7273
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
            "detectMoves": false
        }
    });
    for (key, want) in expected.as_object().unwrap() {
        assert_eq!(&value[key], want, "field {key}");
    }
    assert_eq!(value["left"]["lineTerminator"], "lf");
    assert_eq!(value["right"]["finalNewline"], false);
    assert_eq!(value["right"]["lineCount"], 6);
    assert_eq!(
        d.id,
        "sha256:488c0d1086c1e8389d24959f51ccf6282d9075b0c715daa4bbe7649fe165ea82"
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
fn invalid_utf8_is_an_error_document() {
    let d = doc(b"\xEF\xBB\xBFok\n\xff\n", b"ok", "{}");
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
    assert_eq!(doc(b"\xC0\xAF", b"", "{}").status, Status::Error);
    assert_eq!(doc(b"\xED\xA0\x80", b"", "{}").status, Status::Error);
    // UTF-16 with a BOM fails the UTF-8 check.
    assert_eq!(doc(b"\xFF\xFEa\x00", b"", "{}").status, Status::Error);
}

#[test]
fn unsupported_options_give_error_documents() {
    for options in [
        r#"{"ignore": {"blankLines": true}}"#,
        r#"{"ignore": {"regex": ["\\d+"]}}"#,
        r#"{"detectMoves": true}"#,
    ] {
        let d = doc(b"a\n", b"b\n", options);
        assert_eq!(d.status, Status::Error, "{options}");
        assert_eq!(codes(&d), [("unsupported-option", None)]);
        assert!(d.stats.is_none());
        assert_eq!(d.left.line_count, Some(1));
    }
}

#[test]
fn invalid_options_are_rejected_without_a_document() {
    assert!(matches!(
        diff(b"a", b"b", r#"{"bogus": true}"#),
        Err(Error::InvalidOptions(_))
    ));
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
}

#[test]
fn input_names_keep_the_final_component() {
    assert_eq!(Input::named(b"", "a/b/c.txt").name(), Some("c.txt"));
    assert_eq!(Input::named(b"", r"C:\x\y.txt").name(), Some("y.txt"));
    assert_eq!(Input::named(b"", "dir/").name(), None);
    assert_eq!(Input::new(b"").name(), None);
}
