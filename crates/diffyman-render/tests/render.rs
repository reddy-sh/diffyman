//! Behavior of the renderers: byte-exact patches, regrouping, escaping,
//! limits and robustness against documents that do not fit their inputs.

use std::fs;
use std::path::{Path, PathBuf};

use diffyman::Input;
use diffyman::model::{DiffDocument, OpKind, Options, Range, Side, Span};
use diffyman_render::{self as render, HtmlOptions, HtmlView, RenderError, UnifiedOptions};

fn doc(left: &[u8], right: &[u8], options: &str) -> DiffDocument {
    doc_named(left, "x.txt", right, "x.txt", options)
}

fn doc_named(left: &[u8], ln: &str, right: &[u8], rn: &str, options: &str) -> DiffDocument {
    let options = diffyman::parse_options(options).unwrap();
    diffyman::diff_document(Input::named(left, ln), Input::named(right, rn), &options).unwrap()
}

fn unified(left: &[u8], right: &[u8], options: &str) -> String {
    let d = doc(left, right, options);
    render::unified(&d, left, right, &UnifiedOptions::default()).unwrap()
}

fn html(d: &DiffDocument, left: &[u8], right: &[u8]) -> String {
    render::html(d, left, right, &HtmlOptions::default()).unwrap()
}

#[test]
fn unified_keeps_crlf_bytes() {
    assert_eq!(
        unified(b"a\r\nb\r\n", b"a\r\nc\r\n", "{}"),
        "--- a/x.txt\n+++ b/x.txt\n@@ -1,2 +1,2 @@\n a\r\n-b\r\n+c\r\n"
    );
    // LF to CRLF is a change under whitespace `none`.
    assert_eq!(
        unified(b"a\n", b"a\r\n", "{}"),
        "--- a/x.txt\n+++ b/x.txt\n@@ -1 +1 @@\n-a\n+a\r\n"
    );
}

#[test]
fn unified_marks_missing_final_newline() {
    assert_eq!(
        unified(b"a\nb", b"a\nc", "{}"),
        concat!(
            "--- a/x.txt\n+++ b/x.txt\n@@ -1,2 +1,2 @@\n a\n",
            "-b\n\\ No newline at end of file\n+c\n\\ No newline at end of file\n"
        )
    );
    assert_eq!(
        unified(b"a", b"a\n", "{}"),
        "--- a/x.txt\n+++ b/x.txt\n@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+a\n"
    );
}

#[test]
fn unified_edge_ranges() {
    assert_eq!(
        unified(b"", b"a\nb\n", "{}"),
        "--- a/x.txt\n+++ b/x.txt\n@@ -0,0 +1,2 @@\n+a\n+b\n"
    );
    assert_eq!(
        unified(b"a\nb\n", b"", "{}"),
        "--- a/x.txt\n+++ b/x.txt\n@@ -1,2 +0,0 @@\n-a\n-b\n"
    );
    assert_eq!(
        unified(b"a\nb\n", b"a\nb\nc\n", r#"{"context": 0}"#),
        "--- a/x.txt\n+++ b/x.txt\n@@ -2,0 +3 @@\n+c\n"
    );
    assert_eq!(unified(b"same\n", b"same\n", "{}"), "");
}

#[test]
fn unified_bom_and_lone_cr() {
    assert_eq!(
        unified(
            "\u{feff}a\nb\n".as_bytes(),
            "\u{feff}a\nc\n".as_bytes(),
            "{}"
        ),
        "--- a/x.txt\n+++ b/x.txt\n@@ -1,2 +1,2 @@\n \u{feff}a\n-b\n+c\n"
    );
    assert_eq!(
        unified(b"a\rb\r", b"a\rc\r", "{}"),
        "--- a/x.txt\n+++ b/x.txt\n@@ -1,2 +1,2 @@\n a\r\n-b\r\n+c\r\n"
    );
}

#[test]
fn unified_context_lines_come_from_the_left() {
    let out = unified(
        b"a  x\nb\nc\n",
        b"a x\nB\nc\n",
        r#"{"ignore": {"whitespace": "change"}}"#,
    );
    assert_eq!(
        out,
        "--- a/x.txt\n+++ b/x.txt\n@@ -1,3 +1,3 @@\n a  x\n-b\n+B\n c\n"
    );
}

#[test]
fn unified_labels_color_and_escapes() {
    let (l, r) = (
        b"keep\nold \x1b[2J\n".as_slice(),
        b"keep\nnew \xe2\x80\xae\n".as_slice(),
    );
    let d = doc_named(l, "we\"ird\nname", r, "plain.txt", "{}");
    let plain = render::unified(&d, l, r, &UnifiedOptions::default()).unwrap();
    assert!(
        plain.starts_with("--- \"a/we\\\"ird\\nname\"\n+++ b/plain.txt\n"),
        "{plain}"
    );
    assert!(plain.contains("-old \x1b[2J\n"), "patches stay byte-exact");
    assert!(!plain.contains("\x1b[m"));

    let shown = render::unified(
        &d,
        l,
        r,
        &UnifiedOptions {
            color: true,
            escape_controls: true,
            left_label: Some("old".to_owned()),
            right_label: Some("new".to_owned()),
            ..UnifiedOptions::default()
        },
    )
    .unwrap();
    assert_eq!(
        shown,
        concat!(
            "\x1b[1m--- old\x1b[m\n\x1b[1m+++ new\x1b[m\n\x1b[36m@@ -1,2 +1,2 @@\x1b[m\n",
            " keep\n\x1b[31m-old <ESC>[2J\x1b[m\n\x1b[32m+new <RLO>\x1b[m\n"
        )
    );
}

#[test]
fn binary_documents_render_like_git() {
    let (l, r) = (
        b"\x00\x01\x02\x03".as_slice(),
        b"\x00\x01\xff\x03\x04".as_slice(),
    );
    let d = doc_named(l, "a.bin", r, "b.bin", "{}");
    assert_eq!(
        render::unified(&d, l, r, &UnifiedOptions::default()).unwrap(),
        "Binary files a/a.bin and b/b.bin differ\n"
    );
    assert_eq!(
        render::summary_line(&d),
        "a.bin -> b.bin: different, 1 changed range, +3 -2 bytes, 44.44% similar"
    );
    let summary = render::summary(&d, &render::SummaryOptions::default());
    assert!(
        summary.contains("  left:     application/octet-stream, 4 bytes\n"),
        "{summary}"
    );
    assert!(
        summary.contains("  bytes:    3 added, 2 removed, 2 unchanged\n"),
        "{summary}"
    );
    let same = doc_named(l, "a.bin", l, "b.bin", "{}");
    assert_eq!(
        render::unified(&same, l, l, &UnifiedOptions::default()).unwrap(),
        ""
    );
    // The inputs must be the ones the document describes.
    assert_eq!(
        render::unified(&d, r, r, &UnifiedOptions::default()),
        Err(RenderError::InputMismatch("left"))
    );
    // HTML lists the range with a hex dump of both sides.
    let page = html(&d, l, r);
    assert_safe(&page);
    assert!(
        page.contains("1 changed range: 3 bytes added, 2 removed, 2 unchanged; 44.44% similar.")
    );
    assert!(page.contains("00 01 <del>02 03</del>"), "{page}");
    assert!(page.contains("00 01 <ins>ff 03 04</ins>"), "{page}");
    assert!(page.contains("|..<del>..</del>"), "{page}");
    assert!(page.contains("application/octet-stream"));
    assert_eq!(
        render::html(&d, r, r, &HtmlOptions::default()),
        Err(RenderError::InputMismatch("left"))
    );
    let page = html(&same, l, l);
    assert!(page.contains("byte for byte the same") && page.contains("No changes to show."));
}

#[test]
fn hex_dumps_are_bounded_and_ranges_checked() {
    let left: Vec<u8> = (0..8192u32).map(|i| (i * 7 % 251) as u8).collect();
    let mut right = left.clone();
    for at in (0..8192).step_by(512) {
        right[at] ^= 0xff;
        right[at + 1] = b'<';
    }
    let d = doc_named(&left, "a.bin", &right, "b.bin", "{}");
    assert_eq!(d.bytes.len(), 16);
    let page = render::html(
        &d,
        &left,
        &right,
        &HtmlOptions {
            max_hex_bytes: 200,
            ..HtmlOptions::default()
        },
    )
    .unwrap();
    assert_safe(&page);
    // 16 bytes per row and side: six rows fit 200 bytes, and the rest of the
    // ranges are listed without a dump.
    assert_eq!(page.matches("<span class=\"ho\">").count(), 12);
    assert!(page.contains("Hex dumps cut short."));
    assert!(page.contains("&lt;</ins>"), "escaped in the text column");
    let page = render::html(
        &d,
        &left,
        &right,
        &HtmlOptions {
            // 4 KiB past the header reserve: a few ranges with their dumps.
            max_bytes: 100 * 1024,
            ..HtmlOptions::default()
        },
    )
    .unwrap();
    assert_safe(&page);
    assert!(page.contains("<strong>Truncated.</strong>"));
    assert!(page.contains(" of 16 byte ranges are listed"));
    assert!(page.len() < 100 * 1024);
    // Ranges outside the input, or with the wrong shape, are rejected.
    let mut broken = d.clone();
    broken.bytes[3].right_length = u64::MAX;
    let mut wrong = d.clone();
    wrong.bytes[0].op = OpKind::Insert;
    for b in [&broken, &wrong] {
        assert!(matches!(
            render::html(b, &left, &right, &HtmlOptions::default()),
            Err(RenderError::InvalidDocument(_))
        ));
    }
}

/// The document of a corpus case, from its files or folders.
fn corpus_doc(case: &Path) -> (DiffDocument, Vec<u8>, Vec<u8>) {
    let options =
        diffyman::parse_options(&fs::read_to_string(case.join("options.json")).unwrap()).unwrap();
    let pick = |stem: &str| {
        let dir = case.join(stem);
        if dir.is_dir() {
            return dir;
        }
        fs::read_dir(case)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(&format!("{stem}."))
            })
            .unwrap()
    };
    let (l, r) = (pick("left"), pick("right"));
    if l.is_dir() || r.is_dir() {
        let d = diffyman::diff_paths(&l, &r, &options, &diffyman::Settings::default()).unwrap();
        return (d, Vec::new(), Vec::new());
    }
    let (lb, rb) = (fs::read(&l).unwrap(), fs::read(&r).unwrap());
    let name = |p: &Path| p.file_name().unwrap().to_string_lossy().into_owned();
    let (ln, rn) = (name(&l), name(&r));
    let d =
        diffyman::diff_document(Input::named(&lb, &ln), Input::named(&rb, &rn), &options).unwrap();
    (d, lb, rb)
}

/// Contents of top-level entries of a corpus case whose sides are folders.
fn folder_contents(case: &Path) -> impl FnMut(&[&str]) -> Option<(Vec<u8>, Vec<u8>)> + '_ {
    move |path: &[&str]| {
        let [entry] = path else {
            return None;
        };
        Some((
            fs::read(case.join("left").join(entry)).ok()?,
            fs::read(case.join("right").join(entry)).ok()?,
        ))
    }
}

#[test]
fn html_renders_every_corpus_case_safely() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let mut rendered = 0;
    for group in ["text", "moved", "binary", "tree"] {
        let mut cases: Vec<PathBuf> = fs::read_dir(root.join(group))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.join("options.json").exists())
            .collect();
        cases.sort();
        for case in cases {
            let (d, l, r) = corpus_doc(&case);
            let mut contents = folder_contents(&case);
            let page =
                render::html_with_contents(&d, &l, &r, &mut contents, &HtmlOptions::default())
                    .unwrap_or_else(|e| panic!("{}: {e}", case.display()));
            assert_safe(&page);
            assert!(page.ends_with("</html>\n"), "{}", case.display());
            rendered += 1;
        }
    }
    assert!(rendered >= 130, "{rendered}");
}

#[test]
fn tree_html_lists_entries_and_children() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/tree");
    // Two zips: no entry content, so the text child is listed by position.
    let (d, ..) = corpus_doc(&root.join("011-zip-changes"));
    let page = html(&d, &[], &[]);
    assert_safe(&page);
    assert!(page.contains("6 changed entries: 2 added, 1 removed, 2 modified, 1 moved"));
    assert!(page.contains("old/name.txt <span aria-hidden=\"true\">→</span><span class=\"sr\">moved to</span> new/name.txt"));
    assert!(page.contains("symbolic link <span aria-hidden=\"true\">→</span><span class=\"sr\">to</span> <code>src/app.js</code>"));
    assert!(page.contains("<summary>Text: different, 1 change in 1 hunk"));
    assert!(
        page.contains("<li><code>@@ -1,2 +1,3 @@</code> 1 line removed, 2 added</li>"),
        "{page}"
    );
    assert!(page.contains("Positions only."));
    assert!(page.contains("<summary>3 unchanged entries</summary>"));

    // Two folders with their contents: real lines and a hex dump.
    let case = root.join("002-folder-changes");
    let (d, ..) = corpus_doc(&case);
    let mut contents = folder_contents(&case);
    let page =
        render::html_with_contents(&d, &[], &[], &mut contents, &HtmlOptions::default()).unwrap();
    assert_safe(&page);
    assert!(page.contains("<table class=\"diff emb\">"));
    assert!(page.contains("<code class=\"hex\">"));
    assert!(page.contains("id=\"c1-u-h1\"") && page.contains("id=\"c2-u-h1\""));
    assert!(!page.contains("Positions only."));

    // Contents that do not match are not shown.
    let mut wrong = |_: &[&str]| Some((b"x\n".to_vec(), b"y\n".to_vec()));
    let page =
        render::html_with_contents(&d, &[], &[], &mut wrong, &HtmlOptions::default()).unwrap();
    assert_safe(&page);
    assert!(page.contains("Content mismatch."));
    assert!(!page.contains("<table class=\"diff emb\">"));

    // The content budget and the depth cap.
    let mut asked = 0;
    let mut counting = |path: &[&str]| {
        asked += 1;
        folder_contents(&case)(path)
    };
    let page = render::html_with_contents(
        &d,
        &[],
        &[],
        &mut counting,
        &HtmlOptions {
            max_content_bytes: 10,
            ..HtmlOptions::default()
        },
    )
    .unwrap();
    assert_eq!(asked, 0);
    assert!(page.contains("Content limit."));
    let (nested, ..) = corpus_doc(&root.join("018-nested-zip"));
    let deep = render::html(&nested, &[], &[], &HtmlOptions::default()).unwrap();
    assert!(
        deep.contains("<summary>Archive: different") && deep.contains("<summary>Text: different")
    );
    let shallow = render::html(
        &nested,
        &[],
        &[],
        &HtmlOptions {
            max_depth: 1,
            ..HtmlOptions::default()
        },
    )
    .unwrap();
    assert_safe(&shallow);
    assert!(shallow.contains("deeper than this report's depth limit"));
    assert!(shallow.contains("Nesting."));

    // Warnings that matter for trees are flagged.
    let (d, ..) = corpus_doc(&root.join("014-zip-traversal-names"));
    let page = html(&d, &[], &[]);
    assert!(page.contains("<strong>Unsafe paths (left).</strong>"));
    assert_eq!(
        page.matches("<span class=\"badge bad\">unsafe path</span>")
            .count(),
        // The legend, the notice and one per unsafe entry.
        2 + d
            .tree
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.path.starts_with('/') || e.path.contains("..") || e.path.contains(':'))
            .count()
    );
    let (d, ..) = corpus_doc(&root.join("017-zip-encrypted"));
    let page = html(&d, &[], &[]);
    assert!(page.contains("<strong>Partial.</strong>"));
    assert!(page.contains("<strong>Encrypted entries (left).</strong>"));
    assert!(page.contains("<span class=\"badge\">content not read</span>"));
}

#[test]
fn tree_html_escapes_hostile_paths_and_is_capped() {
    let case =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/tree/011-zip-changes");
    let (mut d, ..) = corpus_doc(&case);
    let hostile = "<img src=x onerror=alert(1)>\u{202e}gpj.exe\"'&";
    let tree = d.tree.as_mut().unwrap();
    for (i, e) in tree.entries.iter_mut().enumerate() {
        e.path = format!("{hostile}/{i}");
        if e.from_path.is_some() {
            e.from_path = Some(format!("../{hostile}"));
        }
        for side in [e.left.as_mut(), e.right.as_mut()].into_iter().flatten() {
            if side.target.is_some() {
                side.target = Some(format!("</code><script>{hostile}"));
            }
            if let Some(sha) = &mut side.sha256 {
                sha.insert_str(0, "\"><b>\u{2066}");
            }
        }
    }
    let page = html(&d, &[], &[]);
    assert_safe(&page);
    assert!(!page.contains("<img") && !page.contains("<b>") && !page.contains("<script"));
    assert!(!page.contains('\u{202e}') && !page.contains('\u{2066}'));
    assert!(page.contains("Bidirectional control characters."));
    assert!(page.contains("<span class=\"badge bad\">unsafe path</span>"));

    // Many entries: the size cap holds, and the page says so.
    let entry = d.tree.as_ref().unwrap().entries[0].clone();
    d.tree.as_mut().unwrap().entries = (0..20_000)
        .map(|i| diffyman::model::TreeEntry {
            path: format!("dir/file-{i:05}.txt"),
            ..entry.clone()
        })
        .collect();
    let max = 300_000;
    let page = render::html(
        &d,
        &[],
        &[],
        &HtmlOptions {
            max_bytes: max,
            ..HtmlOptions::default()
        },
    )
    .unwrap();
    assert!(page.len() < max + 4096, "{}", page.len());
    assert!(page.contains("Truncated"));
    assert_safe(&page);
}

#[test]
fn unified_error_documents() {
    let d = doc(b"ok\n", b"caf\xe9\n", "{}");
    assert_eq!(
        render::unified(&d, b"ok\n", b"caf\xe9\n", &UnifiedOptions::default()).unwrap(),
        "Binary files a/x.txt and b/x.txt differ\n"
    );
    let d = doc(b"a\n", b"b\n", r#"{"ignore": {"regex": ["("]}}"#);
    assert_eq!(
        render::unified(&d, b"a\n", b"b\n", &UnifiedOptions::default()),
        Err(RenderError::ErrorDocument("invalid-regex".to_owned()))
    );
    // HTML and summary still describe error documents.
    assert!(html(&d, b"a\n", b"b\n").contains("invalid-regex"));
    assert_eq!(
        render::summary_line(&d),
        "x.txt -> x.txt: error: invalid-regex"
    );
}

#[test]
fn inputs_must_match_the_document() {
    let d = doc(b"a\n", b"b\n", "{}");
    assert_eq!(
        render::unified(&d, b"z\n", b"b\n", &UnifiedOptions::default()),
        Err(RenderError::InputMismatch("left"))
    );
    assert_eq!(
        render::html(&d, b"a\n", b"bb\n", &HtmlOptions::default()),
        Err(RenderError::InputMismatch("right"))
    );
}

fn named<'a>(bytes: &'a [u8], name: Option<&'a str>) -> Input<'a> {
    match name {
        Some(n) => Input::named(bytes, n),
        None => Input::new(bytes),
    }
}

/// For every corpus case on files, rendering from the engine's decoded
/// inputs (`*_computed`) gives exactly the output of the checked path, with
/// and without the engine's text, and in every unified style.
#[test]
fn computed_inputs_render_like_checked_inputs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let mut compared = 0;
    for group in ["text", "moved", "binary", "tree"] {
        let mut cases: Vec<PathBuf> = fs::read_dir(root.join(group))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.join("options.json").exists())
            .collect();
        cases.sort();
        for case in cases {
            let (d, l, r) = corpus_doc(&case);
            if d.kind == diffyman::model::Kind::Tree && l.is_empty() {
                continue; // folders: no input bytes
            }
            let options = d.options.clone();
            let settings = diffyman::Settings::default();
            let (ln, rn) = (d.left.name.clone(), d.right.name.clone());
            let (with_text, text) = diffyman::diff_document_with_text(
                named(&l, ln.as_deref()),
                named(&r, rn.as_deref()),
                &options,
                &settings,
            )
            .unwrap();
            assert_eq!(with_text.id, d.id, "{}", case.display());
            if d.kind == diffyman::model::Kind::Text {
                assert_eq!(
                    text.is_some(),
                    d.left.encoding.is_some() && d.right.encoding.is_some(),
                    "{}",
                    case.display()
                );
            } else {
                assert!(text.is_none(), "{}", case.display());
            }
            for computed in [
                render::Computed {
                    left: &l,
                    right: &r,
                    text: text.as_ref(),
                },
                render::Computed {
                    left: &l,
                    right: &r,
                    text: None,
                },
            ] {
                for (color, escape) in [(false, false), (true, true)] {
                    let o = UnifiedOptions {
                        color,
                        escape_controls: escape,
                        ..UnifiedOptions::default()
                    };
                    assert_eq!(
                        render::unified_computed(&d, &computed, &o),
                        render::unified(&d, &l, &r, &o),
                        "{}",
                        case.display()
                    );
                }
                let o = HtmlOptions::default();
                assert_eq!(
                    render::html_computed(&d, &computed, &o),
                    render::html(&d, &l, &r, &o),
                    "{}",
                    case.display()
                );
            }
            compared += 1;
        }
    }
    assert!(compared >= 110, "{compared}");
}

/// The computed path still checks what is cheap: sizes, line counts, and
/// that the engine's text belongs to the document.
#[test]
fn computed_inputs_are_checked_cheaply() {
    let (l, r) = (b"a\nb\n".as_slice(), b"a\nc\n".as_slice());
    let (d, text) = diffyman::diff_document_with_text(
        Input::new(l),
        Input::new(r),
        &Options::default(),
        &diffyman::Settings::default(),
    )
    .unwrap();
    let text = text.unwrap();
    let wrong_size = render::Computed {
        left: b"a\nb\nc\n",
        right: r,
        text: Some(&text),
    };
    assert_eq!(
        render::unified_computed(&d, &wrong_size, &UnifiedOptions::default()),
        Err(RenderError::InputMismatch("left"))
    );
    // Same size, other text: not hashed, so this renders from the given
    // text; a text that does not fit the document is refused.
    let other = render::TextInputs {
        left: render::IndexedText::new("a\nb\n\n"),
        right: text.right.clone(),
    };
    let misfit = render::Computed {
        left: l,
        right: r,
        text: Some(&other),
    };
    assert!(matches!(
        render::unified_computed(&d, &misfit, &UnifiedOptions::default()),
        Err(RenderError::InvalidDocument(_))
    ));
    let mut broken = d.clone();
    broken.hunks[0].ops[0].left_range = Range { start: 9, count: 1 };
    let ok = render::Computed {
        left: l,
        right: r,
        text: Some(&text),
    };
    assert!(matches!(
        render::html_computed(&broken, &ok, &HtmlOptions::default()),
        Err(RenderError::InvalidDocument(_))
    ));
}

#[test]
fn documents_that_break_the_rules_are_rejected_without_panics() {
    let (l, r) = (b"a\nb\nc\n".as_slice(), b"a\nB\nc\n".as_slice());
    let good = doc(l, r, "{}");
    let mut broken = Vec::new();
    let mut d = good.clone();
    d.hunks[0].ops[1].left_range = Range {
        start: 99,
        count: 5,
    };
    broken.push(d);
    let mut d = good.clone();
    d.hunks[0].ops[1].right_range.count = 0;
    broken.push(d);
    let mut d = good.clone();
    d.hunks[0].ops[1].op = OpKind::Unknown;
    broken.push(d);
    let mut d = good.clone();
    d.hunks[0].ops.clear();
    broken.push(d);
    let mut d = good.clone();
    d.hunks.push(d.hunks[0].clone());
    broken.push(d);
    let mut d = good.clone();
    d.left.line_count = Some(7);
    broken.push(d);
    for d in &broken {
        for result in [
            render::unified(d, l, r, &UnifiedOptions::default()),
            render::html(d, l, r, &HtmlOptions::default()),
        ] {
            assert!(
                matches!(result, Err(RenderError::InvalidDocument(_))),
                "{result:?}"
            );
        }
    }
    // Spans and moves that point nowhere are ignored, not trusted.
    let mut d = doc(l, r, r#"{"granularity": "char", "detectMoves": true}"#);
    d.hunks[0].ops[1].inline = vec![
        Span {
            side: Side::Left,
            line: 2,
            start: 5,
            end: u64::MAX,
            ..Span::default()
        },
        Span {
            side: Side::Right,
            line: 999,
            start: 0,
            end: 1,
            op: OpKind::Insert,
        },
    ];
    d.moves = vec![diffyman::model::Move {
        from: diffyman::model::MoveEnd {
            side: Side::Left,
            range: Range {
                start: 1,
                count: u64::MAX,
            },
        },
        ..Default::default()
    }];
    let page = html(&d, l, r);
    assert!(page.ends_with("</html>\n"));
}

/// Regrouping a document's ops with another context gives the hunks the
/// engine computes with that context.
#[test]
fn regrouping_matches_the_engine() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let mut checked = 0;
    for group in ["text", "moved"] {
        let mut cases: Vec<PathBuf> = fs::read_dir(root.join(group))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.join("options.json").exists())
            .collect();
        cases.sort();
        for case in cases {
            let pick = |stem: &str| {
                fs::read_dir(&case)
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .find(|p| {
                        p.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with(&format!("{stem}."))
                    })
                    .unwrap()
            };
            let (l, r) = (
                fs::read(pick("left")).unwrap(),
                fs::read(pick("right")).unwrap(),
            );
            let base =
                diffyman::parse_options(&fs::read_to_string(case.join("options.json")).unwrap())
                    .unwrap();
            let at = |context: u32| {
                let options = Options {
                    context,
                    ..base.clone()
                };
                diffyman::diff_document(Input::new(&l), Input::new(&r), &options).unwrap()
            };
            let own = at(base.context);
            if own.status != diffyman::model::Status::Different {
                continue;
            }
            for n in [0, 1, 2, 3, 4, 6, 10] {
                let regrouped = render::unified(
                    &own,
                    &l,
                    &r,
                    &UnifiedOptions {
                        context: Some(n),
                        ..UnifiedOptions::default()
                    },
                )
                .unwrap();
                if n > base.context && base.ignore.blank_lines {
                    continue; // context cannot grow past ignored ops the document does not list
                }
                let direct = render::unified(&at(n), &l, &r, &UnifiedOptions::default()).unwrap();
                assert_eq!(regrouped, direct, "{} with context {n}", case.display());
                checked += 1;
            }
        }
    }
    assert!(checked > 300, "{checked}");
}

/// Every tag and attribute in the report, as `(tag, [(attribute, value)])`.
/// Content is escaped, so every `<` starts a real tag.
fn tags(page: &str) -> Vec<(String, Vec<(String, String)>)> {
    let body = page.split("</style>").nth(1).expect("one style element");
    let mut out = Vec::new();
    for chunk in body.split('<').skip(1) {
        let tag = &chunk[..chunk.find('>').expect("closed tag")];
        let tag = tag.trim_start_matches('/');
        let name: String = tag
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        let mut attrs = Vec::new();
        let mut rest = &tag[name.len()..];
        while let Some(eq) = rest.find('=') {
            let attr = rest[..eq].trim().to_owned();
            let after = &rest[eq + 1..];
            assert!(after.starts_with('"'), "unquoted attribute in <{tag}>");
            let end = after[1..].find('"').unwrap() + 1;
            attrs.push((attr, after[1..end].to_owned()));
            rest = &after[end + 1..];
        }
        out.push((name, attrs));
    }
    out
}

fn assert_safe(page: &str) {
    const TAGS: &[&str] = &[
        "html", "head", "meta", "title", "style", "body", "header", "h1", "h2", "p", "span",
        "strong", "ul", "ol", "li", "details", "summary", "table", "thead", "tbody", "tr", "th",
        "td", "code", "main", "input", "label", "section", "colgroup", "col", "del", "ins", "a",
        "footer", "!DOCTYPE",
    ];
    const ATTRS: &[&str] = &[
        "class",
        "id",
        "href",
        "title",
        "lang",
        "charset",
        "name",
        "content",
        "type",
        "for",
        "colspan",
        "scope",
        "role",
        "aria-label",
        "aria-labelledby",
        "aria-hidden",
    ];
    assert_eq!(page.matches("<style>").count(), 1);
    assert!(!page.contains("<script"), "no script at all");
    let mut ids = std::collections::BTreeSet::new();
    let mut hrefs = Vec::new();
    for (tag, attrs) in tags(page) {
        let tag = if tag.is_empty() {
            "!DOCTYPE".to_owned()
        } else {
            tag
        };
        assert!(TAGS.contains(&tag.as_str()), "unexpected tag <{tag}>");
        for (attr, value) in attrs {
            let attr = attr
                .split_whitespace()
                .last()
                .unwrap_or_default()
                .to_owned();
            assert!(
                ATTRS.contains(&attr.as_str()),
                "unexpected attribute {attr} on <{tag}>"
            );
            match attr.as_str() {
                "id" => assert!(ids.insert(value.clone()), "duplicate id {value}"),
                "href" => hrefs.push(value),
                _ => {}
            }
        }
    }
    for href in hrefs {
        let target = href.strip_prefix('#').expect("only in-page links");
        assert!(ids.contains(target), "dangling link {href}");
    }
}

#[test]
fn html_escapes_hostile_names_and_content() {
    let name = "<img src=x onerror=alert(1)>\"'&\u{202e}gpj.exe";
    let left = b"</script><img src=x onerror=alert(1)>\n<style>body{display:none}</style>\nsame\n";
    let right =
        "</script><img src=x onerror=alert(2)>\n<b onclick=\"x\">\u{202e}evil\u{2066}</b>\nsame\n";
    let d = doc_named(
        left,
        name,
        right.as_bytes(),
        "a\"b.txt",
        r#"{"granularity": "word"}"#,
    );
    for view in [HtmlView::Both, HtmlView::SideBySide, HtmlView::Inline] {
        let page = render::html(
            &d,
            left,
            right.as_bytes(),
            &HtmlOptions {
                view,
                ..HtmlOptions::default()
            },
        )
        .unwrap();
        assert_safe(&page);
        assert!(!page.contains("<img"));
        assert!(!page.contains("<b "));
        assert!(!page.contains('\u{202e}') && !page.contains('\u{2066}'));
        assert!(page.contains("&lt;img src=x onerror=alert(1)&gt;"));
        assert!(page.contains(">RLO</span>") && page.contains(">LRI</span>"));
        assert!(page.contains("Bidirectional control characters"));
        assert_eq!(page.contains("id=\"view-split\""), view == HtmlView::Both);
    }
    let summary = render::summary_line(&d);
    assert!(!summary.contains('\u{202e}'), "{summary}");
}

#[test]
fn html_marks_moves_spans_and_line_endings() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/moved/001-pure-function-down");
    let (l, r) = (
        fs::read(root.join("left.txt")).unwrap(),
        fs::read(root.join("right.txt")).unwrap(),
    );
    let d = doc(&l, &r, r#"{"detectMoves": true}"#);
    assert!(!d.moves.is_empty());
    let page = html(&d, &l, &r);
    assert_safe(&page);
    assert!(page.contains("Moved blocks"));
    assert!(page.contains("moved to right"));
    assert!(page.contains("⇄<span class=\"sr\"> moved away</span>"));

    // An edited move: its lines show the move's own spans (`moves[].inline`).
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/moved/026-edited-renamed-variable");
    let (l, r) = (
        fs::read(root.join("left.txt")).unwrap(),
        fs::read(root.join("right.txt")).unwrap(),
    );
    let d = doc(&l, &r, r#"{"detectMoves": true, "granularity": "word"}"#);
    let m = &d.moves[0];
    assert!(!m.inline.is_empty());
    let page = html(&d, &l, &r);
    assert_safe(&page);
    let mut without = d.clone();
    without.moves[0].inline.clear();
    let plain_page = html(&without, &l, &r);
    assert!(page.matches("<del>").count() > plain_page.matches("<del>").count());
    assert!(page.matches("<ins>").count() > plain_page.matches("<ins>").count());

    let d = doc(
        b"one two three\n",
        b"one 2 three\n",
        r#"{"granularity": "word"}"#,
    );
    let page = html(&d, b"one two three\n", b"one 2 three\n");
    assert!(page.contains("one <del>two</del> three"));
    assert!(page.contains("one <ins>2</ins> three"));

    let d = doc(b"a\r\nb\r\n", b"a\nb\n", "{}");
    let page = html(&d, b"a\r\nb\r\n", b"a\nb\n");
    assert!(page.contains("title=\"CR LF\">␍␊</span>") && page.contains("title=\"LF\">␊</span>"));
}

#[test]
fn html_size_is_capped() {
    let lines = |prefix: &str| {
        let mut s = String::new();
        for i in 0..20_000 {
            s.push_str(prefix);
            s.push_str(&i.to_string());
            s.push('\n');
        }
        s
    };
    let (left, right) = (lines("left line "), lines("right line "));
    let d = doc(left.as_bytes(), right.as_bytes(), r#"{"context": 0}"#);
    let max = 300_000;
    let page = render::html(
        &d,
        left.as_bytes(),
        right.as_bytes(),
        &HtmlOptions {
            max_bytes: max,
            ..HtmlOptions::default()
        },
    )
    .unwrap();
    assert!(page.len() < max + 4096, "{}", page.len());
    assert!(page.contains("Truncated"));
    assert!(page.ends_with("</html>\n"));
    assert_safe(&page);

    let long = format!("{}\n", "é".repeat(5000));
    let d = doc(b"short\n", long.as_bytes(), "{}");
    let page = render::html(
        &d,
        b"short\n",
        long.as_bytes(),
        &HtmlOptions {
            max_line_bytes: 101,
            view: HtmlView::Inline,
            ..HtmlOptions::default()
        },
    )
    .unwrap();
    assert!(
        page.contains("… 9898 more bytes not shown"),
        "cut at a char boundary"
    );
    assert!(page.contains("Long lines."));
}

#[test]
fn output_is_deterministic() {
    let (l, r) = (b"a\nb\nc\n".as_slice(), b"a\nx\nc\nd\n".as_slice());
    let mut d1 = doc(l, r, r#"{"granularity": "char", "detectMoves": true}"#);
    let mut d2 = d1.clone();
    d1.provenance.timing.total_micros = 1;
    d2.provenance.timing.total_micros = 987_654;
    d2.provenance.binding = "python".to_owned();
    assert_eq!(html(&d1, l, r), html(&d2, l, r));
    let u = UnifiedOptions::default();
    assert_eq!(
        render::unified(&d1, l, r, &u).unwrap(),
        render::unified(&d2, l, r, &u).unwrap()
    );
}

#[test]
fn summary_lines() {
    let d = doc_named(
        b"a\n",
        "l.txt",
        b"a \n",
        "r.txt",
        r#"{"ignore": {"whitespace": "trailing"}}"#,
    );
    assert_eq!(
        render::summary_line(&d),
        "l.txt -> r.txt: identical, bytes differ only in ignored ways"
    );
    let d = doc_named(
        b"a\nb\n",
        "l.txt",
        b"a\nc",
        "r.txt",
        r#"{"detectMoves": true}"#,
    );
    assert_eq!(
        render::summary(&d, &render::SummaryOptions::default()),
        concat!(
            "l.txt -> r.txt: different, 1 change in 1 hunk, +1 -1 lines, 50% similar\n",
            "  left:     2 lines, 4 bytes, LF\n",
            "  right:    2 lines, 3 bytes, LF, no final newline\n",
            "  lines:    1 added, 1 removed, 1 unchanged\n",
            "  moves:    0 (0 pure, 0 edited), 0 lines moved\n",
            "  warnings: no-newline-at-eof (right)\n",
        )
    );
}

#[test]
fn tree_summaries_count_entries() {
    let case =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/tree/011-zip-changes");
    let (left, right) = (
        fs::read(case.join("left.zip")).unwrap(),
        fs::read(case.join("right.zip")).unwrap(),
    );
    let d = doc_named(&left, "left.zip", &right, "right.zip", "{}");
    assert_eq!(
        render::summary_line(&d),
        "left.zip -> right.zip: different, 6 changed entries \
         (2 added, 1 removed, 2 modified, 1 moved), 40% similar"
    );
    assert_eq!(
        render::summary(&d, &render::SummaryOptions::default()),
        concat!(
            "left.zip -> right.zip: different, 6 changed entries ",
            "(2 added, 1 removed, 2 modified, 1 moved), 40% similar\n",
            "  left:     application/zip, 7 entries, 556 bytes\n",
            "  right:    application/zip, 8 entries, 597 bytes\n",
            "  entries:  2 added, 1 removed, 2 modified, 1 moved, 0 type changes, 3 unchanged\n",
        )
    );
    let same = doc_named(&left, "a.zip", &left, "b.zip", "{}");
    assert_eq!(render::summary_line(&same), "a.zip -> b.zip: identical");
}
