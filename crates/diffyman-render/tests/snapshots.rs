//! Golden snapshots: renderer output for a handful of corpus cases, stored
//! under `tests/snapshots/` and compared byte for byte.
//!
//! After an intended change, rewrite them and review the diff:
//!
//! ```text
//! cargo test -p diffyman-render --test snapshots -- --ignored bless
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use diffyman::Input;
use diffyman_render as render;

/// `(corpus case, formats)`: `u` unified, `h` HTML, `s` summary.
const CASES: &[(&str, &str)] = &[
    ("text/001-identical", "uhs"),
    ("text/004-simple-line-edit", "uhs"),
    ("text/008-no-final-newline", "us"),
    ("text/009-crlf-vs-lf", "uhs"),
    ("text/016-invisible-chars", "uhs"),
    ("text/021-invalid-utf8", "uhs"),
    ("text/024-lone-cr", "us"),
    ("text/033-smart-prose", "uhs"),
    ("text/037-blank-lines-near-changes", "uhs"),
    ("moved/001-pure-function-down", "uhs"),
    ("moved/026-edited-renamed-variable", "hs"),
    ("binary/001-png-one-byte", "uhs"),
    ("binary/002-inserted-bytes", "hs"),
    ("binary/004-identical", "h"),
    ("binary/005-text-vs-binary", "hs"),
    ("binary/008-empty-vs-binary", "h"),
    ("binary/017-dense-overwrites", "h"),
    ("tree/001-identical-folders", "h"),
    ("tree/002-folder-changes", "hs"),
    ("tree/011-zip-changes", "hs"),
    ("tree/014-zip-traversal-names", "hs"),
    ("tree/017-zip-encrypted", "hs"),
    ("tree/018-nested-zip", "h"),
    ("tree/020-zip-corrupt", "hs"),
    ("tree/026-tar-non-utf8-names", "h"),
];

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn snapshots() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

fn input(dir: &Path, stem: &str) -> (Vec<u8>, String) {
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(&format!("{stem}.")))
        })
        .collect();
    found.sort();
    assert_eq!(found.len(), 1, "{}: one {stem}.* file", dir.display());
    let name = found[0].file_name().unwrap().to_string_lossy().into_owned();
    (fs::read(&found[0]).unwrap(), name)
}

/// Every snapshot as `(file name, expected content)`. Folder cases are
/// compared with `diff_paths`, and their HTML shows the entries' content
/// read from the two folders; archive cases have no entry content.
fn render_all() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (case, formats) in CASES {
        let dir = corpus().join(case);
        let options =
            diffyman::parse_options(&fs::read_to_string(dir.join("options.json")).unwrap())
                .unwrap();
        let folders = dir.join("left").is_dir();
        let (doc, left, right) = if folders {
            let doc = diffyman::diff_paths(
                &dir.join("left"),
                &dir.join("right"),
                &options,
                &diffyman::Settings::default(),
            )
            .unwrap();
            (doc, Vec::new(), Vec::new())
        } else {
            let (left, left_name) = input(&dir, "left");
            let (right, right_name) = input(&dir, "right");
            let doc = diffyman::diff_document(
                Input::named(&left, &left_name),
                Input::named(&right, &right_name),
                &options,
            )
            .unwrap();
            (doc, left, right)
        };
        let mut contents = |path: &[&str]| {
            let [entry] = path else {
                return None;
            };
            if !folders {
                return None;
            }
            Some((
                fs::read(dir.join("left").join(entry)).ok()?,
                fs::read(dir.join("right").join(entry)).ok()?,
            ))
        };
        let stem = case.replace('/', "-");
        for format in formats.chars() {
            let (ext, text) = match format {
                'u' => (
                    "diff",
                    render::unified(&doc, &left, &right, &render::UnifiedOptions::default())
                        .unwrap(),
                ),
                'h' => (
                    "html",
                    render::html_with_contents(
                        &doc,
                        &left,
                        &right,
                        &mut contents,
                        &render::HtmlOptions::default(),
                    )
                    .unwrap(),
                ),
                's' => (
                    "txt",
                    render::summary(&doc, &render::SummaryOptions::default()),
                ),
                _ => unreachable!(),
            };
            out.push((format!("{stem}.{ext}"), text));
        }
    }
    out
}

#[test]
fn snapshots_match() {
    let mut failures = Vec::new();
    for (name, actual) in render_all() {
        let path = snapshots().join(&name);
        match fs::read(&path) {
            Ok(expected) if expected == actual.as_bytes() => {}
            Ok(expected) => {
                let at = expected
                    .iter()
                    .zip(actual.as_bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(expected.len().min(actual.len()));
                failures.push(format!("{name}: differs at byte {at}"));
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "snapshots differ (re-bless with `-- --ignored bless` and review):\n{}",
        failures.join("\n")
    );
}

#[test]
#[ignore = "rewrites the snapshot files"]
fn bless() {
    fs::create_dir_all(snapshots()).unwrap();
    for (name, text) in render_all() {
        fs::write(snapshots().join(name), text).unwrap();
    }
}
