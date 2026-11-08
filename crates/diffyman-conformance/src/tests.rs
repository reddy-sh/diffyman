use super::*;

fn range(start: u64, count: u64) -> LabelRange {
    LabelRange { start, count }
}

fn mv(l: (u64, u64), r: (u64, u64), edited: bool) -> MoveSpec {
    MoveSpec {
        left: range(l.0, l.1),
        right: range(r.0, r.1),
        edited,
    }
}

/// A scratch folder under the system temp dir, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("diffyman-conformance-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_case(root: &Path, group: &str, case: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = root.join(group).join(case);
    fs::create_dir_all(&dir).unwrap();
    for (name, text) in files {
        fs::write(dir.join(name), text).unwrap();
    }
    dir
}

#[test]
fn bless_then_pass_then_fail() {
    let scratch = Scratch::new("bless");
    let dir = write_case(
        &scratch.0,
        "text",
        "basic",
        &[
            ("left.txt", "a\nb\n"),
            ("right.txt", "a\nc\n"),
            ("options.json", "{}"),
        ],
    );
    let cases = list_cases(&scratch.0).unwrap();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].0, "text/basic");

    let run = |bless| run_case("text/basic", &dir, &RunOptions { bless }).outcome;
    assert_eq!(run(false), Outcome::New);
    assert_eq!(run(true), Outcome::Blessed);
    let written = fs::read_to_string(dir.join("expected.json")).unwrap();
    assert!(!written.ends_with('\n'));
    assert!(!written.contains("provenance"));
    assert!(written.contains(r#""name":"left.txt""#));
    assert_eq!(run(false), Outcome::Pass);

    // A pretty-printed expected file with provenance still passes.
    let value: Value = serde_json::from_str(&written).unwrap();
    let mut with_prov = value.clone();
    with_prov["provenance"] = serde_json::json!({"anything": 1});
    fs::write(
        dir.join("expected.json"),
        serde_json::to_string_pretty(&with_prov).unwrap(),
    )
    .unwrap();
    assert_eq!(run(false), Outcome::Pass);

    fs::write(dir.join("right.txt"), "a\nd\n").unwrap();
    assert!(matches!(run(false), Outcome::Fail(_)));
}

#[test]
fn broken_cases_are_errors() {
    let scratch = Scratch::new("broken");
    let dir = write_case(
        &scratch.0,
        "text",
        "no-options",
        &[("left.txt", ""), ("right.txt", "")],
    );
    assert!(matches!(
        run_case("text/no-options", &dir, &RunOptions::default()).outcome,
        Outcome::Error(_)
    ));
    let dir = write_case(
        &scratch.0,
        "text",
        "bad-options",
        &[
            ("left.txt", ""),
            ("right.txt", ""),
            ("options.json", r#"{"nope": 1}"#),
        ],
    );
    assert!(matches!(
        run_case("text/bad-options", &dir, &RunOptions::default()).outcome,
        Outcome::Error(_)
    ));
}

#[test]
fn moved_cases_report_metrics() {
    let scratch = Scratch::new("moved");
    // beta() is longer, so the line diff keeps it in place and alpha() moves.
    let alpha = "fn alpha() {\n  one_two_three();\n  four_five_six();\n}\n";
    let beta = "fn beta() {\n  seven_eight();\n  nine_ten();\n  eleven();\n}\n";
    let left = format!("{alpha}{beta}");
    let right = format!("{beta}{alpha}");
    let labels = r#"{"moves": [{"left": {"start": 1, "count": 4}, "right": {"start": 6, "count": 4}, "kind": "pure"}], "notes": "alpha moved down"}"#;
    let dir = write_case(
        &scratch.0,
        "moved",
        "simple",
        &[
            ("left.rs", &left),
            ("right.rs", &right),
            ("options.json", r#"{"detectMoves": true}"#),
            ("labels.json", labels),
        ],
    );
    let result = run_case("moved/simple", &dir, &RunOptions::default());
    assert_eq!(result.outcome, Outcome::New);
    let eval = result.metrics.unwrap();
    assert_eq!(
        eval.all,
        Counts {
            tp: 1,
            fp: 0,
            fn_: 0
        }
    );
    assert_eq!((eval.kind_ok, eval.predicted, eval.labels), (1, 1, 1));
}

#[test]
fn evaluation_gate_and_categories() {
    let labels = [mv((1, 4), (10, 4), false), mv((20, 2), (30, 2), true)];
    let predicted = [mv((1, 4), (10, 4), false), mv((20, 2), (30, 3), false)];
    let e = evaluate(&predicted, &labels);
    assert_eq!(e.all.tp, 2);
    assert_eq!(
        e.pure_only,
        Counts {
            tp: 1,
            fp: 0,
            fn_: 0
        }
    );
    // The second pair matches with the wrong kind and inexact boundaries.
    assert_eq!((e.kind_ok, e.exact), (1, 1));
    assert!(gate_failures(&e, &[]).is_empty());
    let mut total = e;
    total.add(&evaluate(&[mv((50, 3), (60, 3), false)], &[]));
    let failures = gate_failures(&total, &["moved/032-neg-x".to_owned()]);
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert!(failures[0].starts_with("precision"));
    assert!(failures[1].contains("032-neg-x"));
    let missed = evaluate(&[], &labels);
    assert!(
        gate_failures(&missed, &[])
            .iter()
            .any(|f| f.starts_with("recall"))
    );
    assert_eq!(category("moved/001-pure-function-down"), "pure");
    assert_eq!(category("moved/040-neg-repeated-short-lines"), "neg");
    assert_eq!(category("odd"), "other");
}

#[test]
fn label_validation() {
    let ok = r#"{"moves": [{"left": {"start": 1, "count": 2}, "right": {"start": 3, "count": 2}, "kind": "edited"}]}"#;
    assert_eq!(parse_labels(ok, 4, 4).unwrap(), [mv((1, 2), (3, 2), true)]);
    for bad in [
        r#"{"moves": [], "extra": 1}"#,
        r#"{"notes": "x"}"#,
        r#"{"moves": [{"left": {"start": 0, "count": 1}, "right": {"start": 1, "count": 1}, "kind": "pure"}]}"#,
        r#"{"moves": [{"left": {"start": 1, "count": 9}, "right": {"start": 1, "count": 1}, "kind": "pure"}]}"#,
        r#"{"moves": [{"left": {"start": 1, "count": 1}, "right": {"start": 1, "count": 1}, "kind": "copy"}]}"#,
        r#"{"moves": [{"left": {"start": 1, "count": 1}, "right": {"start": 1, "count": 1}}]}"#,
        r#"{"moves": [{"left": {"start": 3, "count": 1}, "right": {"start": 1, "count": 1}, "kind": "pure"},
                      {"left": {"start": 1, "count": 1}, "right": {"start": 2, "count": 1}, "kind": "pure"}]}"#,
        r#"{"moves": [{"left": {"start": 1, "count": 2}, "right": {"start": 1, "count": 1}, "kind": "pure"},
                      {"left": {"start": 2, "count": 1}, "right": {"start": 3, "count": 1}, "kind": "pure"}]}"#,
        r#"{"moves": [], "notes": 3}"#,
    ] {
        assert!(parse_labels(bad, 4, 4).is_err(), "accepted {bad}");
    }
    assert!(parse_labels(r#"{"moves": []}"#, 0, 0).unwrap().is_empty());
}

#[test]
fn label_lines_follow_the_document_line_rule() {
    assert_eq!(label_line_count(b""), 0);
    assert_eq!(label_line_count(b"a\nb"), 2);
    assert_eq!(label_line_count(b"a\r\nb\n"), 2);
    // A lone CR ends a line (Diff Document spec §7.2).
    assert_eq!(label_line_count(b"a\rb\n"), 2);
    assert_eq!(label_line_count(b"a\r\n\r"), 2);
}

#[test]
fn matching_and_metrics() {
    let labels = [mv((1, 4), (10, 4), false), mv((20, 2), (30, 2), true)];
    // Exact match, a partial match (Jaccard 3/5 >= 0.5 on both sides) and a miss.
    let predicted = [mv((1, 4), (10, 4), false), mv((50, 3), (60, 3), false)];
    assert_eq!(
        score(&predicted, &labels, MoveMode::Pure),
        Counts {
            tp: 1,
            fp: 1,
            fn_: 0
        }
    );
    assert_eq!(
        score(&predicted, &labels, MoveMode::Edited),
        Counts {
            tp: 1,
            fp: 1,
            fn_: 1
        }
    );

    let partial = [mv((2, 4), (11, 4), false)];
    assert_eq!(score(&partial, &labels[..1], MoveMode::Pure).tp, 1);
    // Jaccard 1/7 on the left: no match.
    let poor = [mv((4, 4), (10, 4), false)];
    assert_eq!(
        score(&poor, &labels[..1], MoveMode::Pure),
        Counts {
            tp: 0,
            fp: 1,
            fn_: 1
        }
    );

    // In pure mode a prediction that matches an edited label is neither TP nor
    // FP; the unmatched pure label is still a false negative.
    let edited_hit = [mv((20, 2), (30, 2), true)];
    assert_eq!(
        score(&edited_hit, &labels, MoveMode::Pure),
        Counts {
            tp: 0,
            fp: 0,
            fn_: 1
        }
    );
    assert_eq!(score(&edited_hit, &labels, MoveMode::Edited).tp, 1);

    // One-to-one: the better prediction wins the label.
    let two = [mv((1, 3), (10, 3), false), mv((1, 4), (10, 4), false)];
    assert_eq!(
        score(&two, &labels[..1], MoveMode::Pure),
        Counts {
            tp: 1,
            fp: 1,
            fn_: 0
        }
    );

    let c = Counts {
        tp: 3,
        fp: 1,
        fn_: 0,
    };
    assert_eq!(fmt_ratio(c.precision()), "0.7500");
    assert_eq!(fmt_ratio(Counts::default().precision()), "n/a");
}

#[test]
fn difference_report() {
    let d = first_difference(r#"{"a":1}"#, r#"{"a":2}"#);
    assert!(d.starts_with("first difference at byte 5"));
    let d = first_difference("{}", "{} ");
    assert!(d.contains("byte 2"));
}
