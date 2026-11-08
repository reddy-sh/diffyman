//! End-to-end tests of the `diffyman` binary: every command, exit codes,
//! color, and golden outputs for a few corpus cases.
//!
//! Golden files live in `tests/golden/`. After an intended change, rewrite
//! them and review the diff:
//!
//! ```text
//! DIFFYMAN_BLESS=1 cargo test -p diffyman-cli --test cli
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn corpus(case: &str) -> PathBuf {
    repo().join("tests/corpus").join(case)
}

/// The two inputs of a corpus case: `left.*` and `right.*`, or the
/// `left` and `right` folders.
fn sides(case: &str) -> (PathBuf, PathBuf) {
    let dir = corpus(case);
    let find = |stem: &str| {
        std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(stem))
            .unwrap_or_else(|| panic!("{case} has no {stem} input"))
    };
    (find("left"), find("right"))
}

/// Runs the binary with `NO_COLOR` unset, so results do not depend on the
/// environment of the test run.
fn diffyman(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_diffyman"))
        .args(args)
        .env_remove("NO_COLOR")
        .output()
        .unwrap()
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// `diffyman diff <left> <right> --options <case options> <extra...>`.
fn diff_case(case: &str, extra: &[&str]) -> Output {
    let (left, right) = sides(case);
    let options = corpus(case).join("options.json");
    let mut args = vec![
        "diff",
        path(&left),
        path(&right),
        "--options",
        path(&options),
    ];
    args.extend_from_slice(extra);
    diffyman(&args)
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// Compares `actual` with `tests/golden/<name>`, or rewrites the file when
/// `DIFFYMAN_BLESS` is set.
fn golden(name: &str, actual: &[u8]) {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("DIFFYMAN_BLESS").is_some() {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, actual).unwrap();
        return;
    }
    let expected = std::fs::read(&file)
        .unwrap_or_else(|_| panic!("missing {}; run with DIFFYMAN_BLESS=1", file.display()));
    assert!(
        expected == actual,
        "{name} differs from the golden file:\n--- expected\n{}\n--- actual\n{}",
        String::from_utf8_lossy(&expected),
        String::from_utf8_lossy(actual)
    );
}

/// The case's `expected.json` and the document `diff --json` prints, both
/// without `provenance`.
fn without_provenance(mut v: Value) -> Value {
    v.as_object_mut().unwrap().remove("provenance");
    v
}

#[test]
fn golden_outputs() {
    let cases: &[(&str, &[&str], &str, i32)] = &[
        (
            "text/004-simple-line-edit",
            &["-f", "unified"],
            "text-004.diff",
            1,
        ),
        (
            "text/004-simple-line-edit",
            &["-f", "summary"],
            "text-004.summary.txt",
            1,
        ),
        (
            "text/004-simple-line-edit",
            &["-f", "html"],
            "text-004.html",
            1,
        ),
        ("text/016-invisible-chars", &[], "text-016.text.txt", 1),
        (
            "text/016-invisible-chars",
            &["-f", "unified"],
            "text-016.diff",
            1,
        ),
        ("binary/002-inserted-bytes", &[], "binary-002.text.txt", 1),
        (
            "binary/002-inserted-bytes",
            &["-f", "summary"],
            "binary-002.summary.txt",
            1,
        ),
        ("tree/002-folder-changes", &[], "tree-002.text.txt", 1),
        (
            "tree/002-folder-changes",
            &["-f", "unified"],
            "tree-002.diff",
            1,
        ),
        ("tree/011-zip-changes", &[], "tree-011.text.txt", 1),
        (
            "tree/011-zip-changes",
            &["-f", "unified"],
            "tree-011.diff",
            1,
        ),
        (
            "tree/011-zip-changes",
            &["-f", "summary"],
            "tree-011.summary.txt",
            1,
        ),
        (
            "binary/002-inserted-bytes",
            &["-f", "html"],
            "binary-002.html",
            1,
        ),
        // Two folders: the modified entries' content is read back, so the
        // report shows their lines and bytes.
        (
            "tree/002-folder-changes",
            &["-f", "html"],
            "tree-002.html",
            1,
        ),
        // Two zips: the entries' changes are listed by position.
        ("tree/011-zip-changes", &["-f", "html"], "tree-011.html", 1),
    ];
    for (case, extra, name, exit) in cases {
        let out = diff_case(case, extra);
        assert_eq!(code(&out), *exit, "{case} {extra:?}: {}", stderr(&out));
        golden(name, &out.stdout);
    }
}

#[test]
fn json_matches_the_corpus() {
    for case in [
        "text/004-simple-line-edit",
        "text/039-regex-invalid",
        "binary/002-inserted-bytes",
        "tree/002-folder-changes",
        "tree/011-zip-changes",
    ] {
        let out = diff_case(case, &["--json"]);
        let text = stdout(&out);
        assert!(
            text.ends_with('\n') && text.matches('\n').count() == 1,
            "{case}"
        );
        let actual: Value = serde_json::from_str(&text).unwrap();
        let expected: Value =
            serde_json::from_slice(&std::fs::read(corpus(case).join("expected.json")).unwrap())
                .unwrap();
        assert_eq!(
            without_provenance(actual.clone()),
            without_provenance(expected),
            "{case}"
        );
        assert_eq!(actual["provenance"]["binding"], "cli");
        let pretty = diff_case(case, &["--pretty"]);
        assert_eq!(code(&pretty), code(&out));
        let pretty: Value = serde_json::from_slice(&pretty.stdout).unwrap();
        assert_eq!(without_provenance(pretty), without_provenance(actual));
    }
}

#[test]
fn exit_codes() {
    let out = diff_case("text/001-identical", &[]);
    assert_eq!(code(&out), 0);
    assert!(out.stdout.is_empty());
    assert_eq!(code(&diff_case("text/004-simple-line-edit", &[])), 1);
    // An error document: invalid regex.
    let out = diff_case("text/039-regex-invalid", &[]);
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("diffyman: error:"),
        "{}",
        stderr(&out)
    );
    // A partial tree is not identical.
    assert_eq!(code(&diff_case("tree/017-zip-encrypted", &[])), 1);
    // Unreadable input, bad options, bad arguments.
    let out = diffyman(&["diff", "/nonexistent/left", "/nonexistent/right"]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("cannot read"), "{}", stderr(&out));
    let (left, right) = sides("text/004-simple-line-edit");
    let out = diffyman(&["diff", path(&left), path(&right), "--context", "4294967296"]);
    assert_eq!(code(&out), 2);
    let out = diffyman(&["diff", path(&left), path(&right), "--algorithm", "quick"]);
    assert_eq!(code(&out), 2);
    assert_eq!(code(&diffyman(&["diff", path(&left)])), 2);
    assert_eq!(code(&diffyman(&["no-such-command"])), 2);
    // HTML covers every kind, and an error document still gets a report
    // (exit code 2).
    let out = diff_case("tree/020-zip-corrupt", &["-f", "html"]);
    assert_eq!(code(&out), 2);
    assert!(stdout(&out).contains("<code>invalid-archive</code> (left)"));
    let out = diff_case("binary/001-png-one-byte", &["-f", "html"]);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("<code class=\"hex\">"));
}

#[test]
fn direct_flags_set_options() {
    let (left, right) = sides("text/007-whitespace-change");
    let plain = diffyman(&["diff", path(&left), path(&right), "--json"]);
    assert_eq!(code(&plain), 1);
    let out = diffyman(&[
        "diff",
        path(&left),
        path(&right),
        "--json",
        "--ignore-whitespace",
        "all",
        "--algorithm",
        "myers",
        "--granularity",
        "word",
        "-U",
        "1",
        "--detect-moves",
        "--position-encoding",
        "utf-16",
        "--ignore-case",
        "--ignore-blank-lines",
        "--ignore-regex",
        r"\d+",
        "--ignore-regex",
        "x",
    ]);
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let options = &doc["options"];
    assert_eq!(options["ignore"]["whitespace"], "all");
    assert_eq!(options["ignore"]["case"], true);
    assert_eq!(options["ignore"]["blankLines"], true);
    assert_eq!(options["ignore"]["regex"], serde_json::json!([r"\d+", "x"]));
    assert_eq!(options["algorithm"], "myers");
    assert_eq!(options["granularity"], "word");
    assert_eq!(options["context"], 1);
    assert_eq!(options["detectMoves"], true);
    assert_eq!(options["positionEncoding"], "utf-16");
    // The flags give the same document as the same options in a file.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("options.json");
    std::fs::write(&file, serde_json::to_vec(options).unwrap()).unwrap();
    let from_file = diffyman(&[
        "diff",
        path(&left),
        path(&right),
        "--json",
        "--options",
        path(&file),
    ]);
    let from_file: Value = serde_json::from_slice(&from_file.stdout).unwrap();
    assert_eq!(from_file["id"], doc["id"]);
}

#[test]
fn color_and_no_color() {
    let (left, right) = sides("text/004-simple-line-edit");
    let run = |color: &str, no_color: Option<&str>| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_diffyman"));
        cmd.args(["diff", path(&left), path(&right), "--color", color]);
        cmd.env_remove("NO_COLOR");
        if let Some(v) = no_color {
            cmd.env("NO_COLOR", v);
        }
        let out = cmd.output().unwrap();
        out.stdout.contains(&0x1b)
    };
    // Output to a pipe: `auto` never colors.
    assert!(!run("auto", None));
    assert!(!run("never", None));
    assert!(run("always", None));
    // An explicit --color=always wins over NO_COLOR (no-color.org).
    assert!(run("always", Some("1")));
    assert!(!run("auto", Some("1")));
    // A colored patch is the plain one plus escape sequences.
    let plain = diffyman(&["diff", path(&left), path(&right), "-f", "unified"]);
    let colored = diffyman(&[
        "diff",
        path(&left),
        path(&right),
        "-f",
        "unified",
        "--color",
        "always",
    ]);
    let stripped = strip_ansi(&stdout(&colored));
    assert_eq!(stripped, stdout(&plain));
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn output_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("out.diff");
    let out = diff_case(
        "text/004-simple-line-edit",
        &["-f", "unified", "-o", path(&file)],
    );
    assert_eq!(code(&out), 1);
    assert!(out.stdout.is_empty());
    let direct = diff_case("text/004-simple-line-edit", &["-f", "unified"]);
    assert_eq!(std::fs::read(&file).unwrap(), direct.stdout);
}

/// `-o` naming an input (directly, by another path, or through a hard
/// link) is refused before anything is written, so the input survives; an
/// output inside a folder input is written after the folder was compared.
#[test]
fn output_file_is_never_an_input() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    let text = "one\ntwo\nthree\n".repeat(1000);
    std::fs::write(&a, &text).unwrap();
    std::fs::write(&b, "one\n2\nthree\n").unwrap();
    let linked = dir.path().join("linked.txt");
    std::fs::hard_link(&a, &linked).unwrap();
    let dotted = dir.path().join(".").join("a.txt");
    for target in [&a, &b, &linked, &dotted] {
        for mmap in [true, false] {
            let mut args = vec![
                "diff",
                path(&a),
                path(&b),
                "-f",
                "summary",
                "-o",
                path(target),
            ];
            if !mmap {
                args.push("--no-mmap");
            }
            let out = diffyman(&args);
            assert_eq!(code(&out), 2, "{}", stderr(&out));
            assert!(stderr(&out).contains("is the input"), "{}", stderr(&out));
            let out = diffyman(&["textconv", path(&a), "-o", path(target)]);
            if target == &b {
                assert_eq!(code(&out), 0, "{}", stderr(&out));
                std::fs::write(&b, "one\n2\nthree\n").unwrap();
            } else {
                assert_eq!(code(&out), 2, "{}", stderr(&out));
                assert!(stderr(&out).contains("is the input"), "{}", stderr(&out));
            }
        }
    }
    assert_eq!(std::fs::read_to_string(&a).unwrap(), text);
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "one\n2\nthree\n");

    // A new file inside a folder input is created after the walk, so it is
    // not one of the compared entries.
    let (left, right) = sides("tree/002-folder-changes");
    let copy = dir.path().join("left");
    copy_dir(&left, &copy);
    let inside = copy.join("diffyman-out.txt");
    let out = diffyman(&["diff", path(&copy), path(&right), "-o", path(&inside)]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    let direct = diffyman(&["diff", path(&left), path(&right)]);
    assert_eq!(std::fs::read(&inside).unwrap(), direct.stdout);
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[test]
fn json_stream() {
    let lines = |case: &str| -> Vec<Value> {
        let out = diff_case(case, &["--json-stream"]);
        stdout(&out)
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    };
    let doc = |case: &str| -> Value {
        serde_json::from_slice(&diff_case(case, &["--json"]).stdout).unwrap()
    };
    let case = "text/017-code-refactor";
    assert_eq!(Value::Array(lines(case)), doc(case)["hunks"]);
    let case = "binary/014-close-and-far-changes";
    assert_eq!(Value::Array(lines(case)), doc(case)["bytes"]);
    let case = "tree/011-zip-changes";
    assert_eq!(Value::Array(lines(case)), doc(case)["tree"]["entries"]);
    assert!(lines("text/001-identical").is_empty());
}

#[test]
fn dir_compares_folders_only() {
    let (left, right) = sides("tree/002-folder-changes");
    let dir = diffyman(&["dir", path(&left), path(&right)]);
    let diff = diffyman(&["diff", path(&left), path(&right)]);
    assert_eq!(code(&dir), 1);
    assert_eq!(dir.stdout, diff.stdout);
    let (left, right) = sides("tree/011-zip-changes");
    let out = diffyman(&["dir", path(&left), path(&right)]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("is not a folder"), "{}", stderr(&out));
}

/// `--max-input-size` refuses a larger input before comparing it, the same
/// way for mapped and read files, folder diffs and `textconv`.
#[test]
fn inputs_over_the_read_limit_are_refused() {
    let (left, right) = sides("text/004-simple-line-edit");
    let size = std::fs::metadata(&left).unwrap().len();
    let within = size
        .max(std::fs::metadata(&right).unwrap().len())
        .to_string();
    let below = (size - 1).to_string();
    let base = ["diff", path(&left), path(&right), "--json"];
    assert_eq!(
        code(&diffyman(
            &[&base[..], &["--max-input-size", &within]].concat()
        )),
        1
    );
    for extra in [&[][..], &["--no-mmap"][..]] {
        let out = diffyman(&[&base[..], &["--max-input-size", &below], extra].concat());
        assert_eq!(code(&out), 2);
        assert!(out.stdout.is_empty());
        let err = stderr(&out);
        assert!(
            err.starts_with("diffyman: error: limit-exceeded: "),
            "{err}"
        );
        assert!(
            err.contains(&format!("is larger than {below} bytes")),
            "{err}"
        );
    }
    let out = diffyman(&["textconv", path(&left), "--max-input-size", "1"]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("limit-exceeded"), "{}", stderr(&out));
    // The archive side of a folder diff.
    let (folder, _) = sides("tree/010-folder-vs-zip");
    let (_, zip) = sides("tree/010-folder-vs-zip");
    let zip_size = std::fs::metadata(&zip).unwrap().len();
    let limit = |n: u64| n.to_string();
    let run = |n: u64| {
        diffyman(&[
            "diff",
            path(&folder),
            path(&zip),
            "--max-input-size",
            &limit(n),
        ])
    };
    let out = run(zip_size - 1);
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    assert!(stderr(&out).contains("limit-exceeded"), "{}", stderr(&out));
    assert_eq!(
        code(&run(zip_size)),
        code(&diffyman(&["diff", path(&folder), path(&zip)]))
    );
    // Sizes take units; nonsense is a usage error.
    assert_eq!(
        code(&diffyman(
            &[&base[..], &["--max-input-size", "1M"]].concat()
        )),
        1
    );
    assert_eq!(
        code(&diffyman(
            &[&base[..], &["--max-input-size", "lots"]].concat()
        )),
        2
    );
}

/// A pipe is read up to the limit and no further: a writer that would go on
/// past it is cut off, and the diff ends with `limit-exceeded`.
#[cfg(unix)]
#[test]
fn pipes_stop_at_the_read_limit() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("fifo");
    let made = Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|s| s.success()) {
        return; // No mkfifo on this system.
    }
    let other = dir.path().join("other.txt");
    std::fs::write(&other, "x\n").unwrap();
    let feed = |len: usize| {
        let fifo = fifo.clone();
        std::thread::spawn(move || {
            use std::io::Write as _;
            let mut writer = std::fs::OpenOptions::new().write(true).open(fifo).unwrap();
            // The reader closes the pipe once it is over its limit.
            let _ = writer.write_all(&vec![b'x'; len]);
        })
    };
    let writer = feed(8 << 20);
    let out = diffyman(&["diff", path(&fifo), path(&other), "--max-input-size", "64K"]);
    writer.join().unwrap();
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(err.contains("limit-exceeded: "), "{err}");
    assert!(err.contains("is larger than 65536 bytes"), "{err}");
    // Within the limit (here the default for pipes) the pipe is compared.
    let writer = feed(1000);
    let out = diffyman(&["diff", path(&fifo), path(&other), "--json"]);
    writer.join().unwrap();
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["left"]["size"], 1000);
}

/// Options files: a key given twice is an error, control characters in
/// messages are escaped, and an oversized file is not read.
#[test]
fn options_files_are_strict() {
    let (left, right) = sides("text/004-simple-line-edit");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("options.json");
    let run = |json: &[u8]| {
        std::fs::write(&file, json).unwrap();
        diffyman(&["diff", path(&left), path(&right), "--options", path(&file)])
    };
    let out = run(br#"{"context": 1, "ignore": {"case": true, "case": false}}"#);
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(err.contains("invalid options: "), "{err}");
    assert!(err.contains(r#"duplicate key "case""#), "{err}");
    for json in [
        &b"{\"\\u001b[2J\\u001b]0;owned\\u0007\": 1}"[..],
        &b"{\"\\u202e\\u001b\": 1, \"\\u202e\\u001b\": 1}"[..],
    ] {
        let out = run(json);
        assert_eq!(code(&out), 2);
        let err = stderr(&out);
        assert!(
            err.starts_with("diffyman: error: invalid options: "),
            "{err}"
        );
        let body = err.trim_end_matches('\n');
        assert!(
            !body.chars().any(|c| c.is_control() || c == '\u{202e}'),
            "raw control character in {err:?}"
        );
        assert!(body.contains("\\u{1b}"), "{err}");
    }
    let mut big = b"{\"ignore\": {\"regex\": [\"".to_vec();
    big.resize(2 << 20, b'a');
    big.extend_from_slice(b"\"]}}");
    let out = run(&big);
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("limit-exceeded: "),
        "{}",
        stderr(&out)
    );
}

#[test]
fn schema_and_formats() {
    let out = diffyman(&["schema"]);
    assert_eq!(code(&out), 0);
    let schema: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(schema["$schema"].as_str().unwrap().contains("2020-12"));
    let out = diffyman(&["schema", "--options"]);
    let schema: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(schema["properties"]["adapter"].is_object(), "{schema}");

    let out = diffyman(&["formats", "--json"]);
    assert_eq!(code(&out), 0);
    let catalog: Value = serde_json::from_slice(&out.stdout).unwrap();
    let kinds: Vec<&str> = catalog["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["text", "binary", "tree"]);
    let out = diffyman(&["formats"]);
    assert_eq!(code(&out), 0);
    let table = stdout(&out);
    for id in [
        "text.histogram@1",
        "binary.bytes@1",
        "tree.entries@1",
        "tar.zst",
    ] {
        assert!(table.contains(id), "{table}");
    }
}

#[test]
fn textconv() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("in.txt");
    std::fs::write(&file, b"\xEF\xBB\xBFOne  Two \r\nrun 17 ms\n").unwrap();
    let out = diffyman(&["textconv", path(&file)]);
    assert_eq!(code(&out), 0);
    assert_eq!(out.stdout, b"One  Two \r\nrun 17 ms\n");
    let out = diffyman(&[
        "textconv",
        path(&file),
        "--ignore-whitespace",
        "change",
        "--ignore-case",
        "--ignore-regex",
        r"\d+",
    ]);
    assert_eq!(stdout(&out), "one two\nrun <masked:0> ms\n");
    // Not UTF-8, and a regex that does not compile: errors.
    let binary = dir.path().join("in.bin");
    std::fs::write(&binary, b"\xFF\xFE\x00").unwrap();
    let out = diffyman(&["textconv", path(&binary)]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("not UTF-8"), "{}", stderr(&out));
    let out = diffyman(&["textconv", path(&file), "--ignore-regex", "("]);
    assert_eq!(code(&out), 2);
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A `git` command that reads and writes only the configuration below
/// `home`: never the user's own.
fn git(home: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    cmd
}

fn install(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_diffyman"))
        .args(["git", "install"])
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

#[test]
fn git_install_prints_by_default_and_writes_on_request() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("gitconfig");
    // The default is a dry run: commands are printed, nothing is written.
    let out = install(home.path(), home.path(), &[]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).contains("Dry run"));
    assert!(stdout(&out).contains(
        r#"git config --global difftool.diffyman.cmd 'diffyman diff "$LOCAL" "$REMOTE"'"#
    ));
    assert!(!config.exists());
    let out = install(home.path(), home.path(), &["--global", "--dry-run"]);
    assert_eq!(code(&out), 0);
    assert!(!config.exists());
    assert_eq!(
        code(&install(home.path(), home.path(), &["--global", "--local"])),
        2
    );
    if !git_available() {
        eprintln!("git not found: skipping the writes");
        return;
    }
    let out = install(home.path(), home.path(), &["--global"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let get = |key: &str| {
        let out = git(home.path())
            .args(["config", "--file", path(&config), "--get", key])
            .output()
            .unwrap();
        stdout(&out).trim_end().to_owned()
    };
    assert_eq!(get("diff.tool"), "diffyman");
    assert_eq!(
        get("difftool.diffyman.cmd"),
        r#"diffyman diff "$LOCAL" "$REMOTE""#
    );
    assert_eq!(get("diff.diffyman.textconv"), "diffyman textconv");
}

#[test]
fn git_difftool_runs_diffyman() {
    if !git_available() {
        eprintln!("git not found: skipping");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let repo = home.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let run = |args: &[&str]| {
        let out = git(home.path())
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", stderr(&out));
        out
    };
    run(&["init", "-q"]);
    run(&["config", "user.name", "Test"]);
    run(&["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("notes.txt"), "alpha\nbeta\n").unwrap();
    run(&["add", "notes.txt"]);
    run(&["commit", "-q", "-m", "first"]);
    std::fs::write(repo.join("notes.txt"), "alpha\ngamma\n").unwrap();
    let program = env!("CARGO_BIN_EXE_diffyman");
    let out = install(home.path(), &repo, &["--local", "--program", program]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    // Nothing went to the global file.
    assert!(!home.path().join("gitconfig").exists());
    let out = run(&["difftool", "--no-prompt", "--", "notes.txt"]);
    let text = stdout(&out);
    assert!(text.contains("-beta\n+gamma\n"), "{text}");
    // The textconv driver, through .gitattributes.
    std::fs::write(repo.join(".gitattributes"), "*.txt diff=diffyman\n").unwrap();
    std::fs::write(repo.join("notes.txt"), "\u{feff}alpha\nbeta\n").unwrap();
    let out = run(&["diff", "--exit-code", "--", "notes.txt"]);
    assert!(out.stdout.is_empty(), "{}", stdout(&out));
}

/// `docs/cli.md` holds the help text of every command after a
/// `<!-- help: ARGS -->` marker. With `DIFFYMAN_BLESS` set the blocks are
/// rewritten from the binary.
#[test]
fn docs_show_the_current_help() {
    let file = repo().join("docs/cli.md");
    let doc = std::fs::read_to_string(&file).unwrap();
    let mut out = String::new();
    let mut lines = doc.split_inclusive('\n');
    let mut checked = 0;
    while let Some(line) = lines.next() {
        out.push_str(line);
        let Some(args) = line
            .trim_end()
            .strip_prefix("<!-- help: ")
            .and_then(|l| l.strip_suffix(" -->"))
        else {
            continue;
        };
        let fence = lines.next().unwrap();
        assert_eq!(
            fence.trim_end(),
            "```text",
            "a help marker needs a text block"
        );
        let mut current = String::new();
        for line in lines.by_ref() {
            if line.trim_end() == "```" {
                break;
            }
            current.push_str(line);
        }
        let help = diffyman(&args.split(' ').collect::<Vec<_>>());
        assert_eq!(code(&help), 0, "{args}");
        let help = stdout(&help);
        if std::env::var_os("DIFFYMAN_BLESS").is_none() {
            assert_eq!(current, help, "docs/cli.md is out of date for `{args}`");
        }
        out.push_str(fence);
        out.push_str(&help);
        out.push_str("```\n");
        checked += 1;
    }
    assert!(checked >= 7, "only {checked} help blocks found");
    if std::env::var_os("DIFFYMAN_BLESS").is_some() {
        std::fs::write(&file, out).unwrap();
    }
}
