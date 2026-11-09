//! Tests of the C ABI through the exported functions themselves: the whole
//! corpus, argument checks, error statuses, ownership and the header.

use std::ffi::{CStr, CString, c_char, c_int};
use std::fs;
use std::path::{Path, PathBuf};
use std::ptr;

use diffyman::model::jcs;
use serde_json::Value;

use super::*;

/// What one call returned, with both strings copied out and freed.
#[derive(Debug)]
struct Outcome {
    status: c_int,
    json: Option<String>,
    error: Option<String>,
}

impl Outcome {
    fn document(&self) -> &str {
        assert_eq!(self.status, DIFFYMAN_OK, "{:?}", self.error);
        assert!(self.error.is_none());
        self.json.as_deref().expect("a document")
    }

    fn failure(&self, status: c_int) -> &str {
        assert_eq!(self.status, status, "{self:?}");
        assert!(self.json.is_none());
        self.error.as_deref().expect("a message")
    }
}

/// Copies out a returned string and frees it.
fn take(s: *mut c_char) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: a string returned by this library, freed once below.
    let text = unsafe { CStr::from_ptr(s) }
        .to_str()
        .expect("UTF-8")
        .to_owned();
    // SAFETY: as above.
    unsafe { diffyman_free_string(s) };
    Some(text)
}

fn as_ptr(s: Option<&CStr>) -> *const c_char {
    s.map_or(ptr::null(), CStr::as_ptr)
}

fn diff(
    left: &[u8],
    right: &[u8],
    options: Option<&CStr>,
    left_name: Option<&CStr>,
    right_name: Option<&CStr>,
) -> Outcome {
    let (mut json, mut error) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: every pointer is valid for the call.
    let status = unsafe {
        diffyman_diff(
            left.as_ptr(),
            left.len(),
            right.as_ptr(),
            right.len(),
            as_ptr(options),
            as_ptr(left_name),
            as_ptr(right_name),
            &raw mut json,
            &raw mut error,
        )
    };
    Outcome {
        status,
        json: take(json),
        error: take(error),
    }
}

fn diff_paths(left: &Path, right: &Path, options: Option<&CStr>, limit: u64) -> Outcome {
    let left = c_string(left.to_str().expect("UTF-8 path"));
    let right = c_string(right.to_str().expect("UTF-8 path"));
    let (mut json, mut error) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: every pointer is valid for the call.
    let status = unsafe {
        diffyman_diff_paths(
            left.as_ptr(),
            right.as_ptr(),
            as_ptr(options),
            limit,
            &raw mut json,
            &raw mut error,
        )
    };
    Outcome {
        status,
        json: take(json),
        error: take(error),
    }
}

fn c_string(text: &str) -> CString {
    CString::new(text).expect("no NUL")
}

/// The conformance bytes: canonical JSON without `provenance`.
fn conformance(text: &str) -> String {
    let mut value: Value = serde_json::from_str(text).expect("JSON");
    if let Value::Object(map) = &mut value {
        map.remove("provenance");
    }
    jcs::canonicalize(&value).expect("canonical")
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

/// The case input `stem.*` (a file) or `stem/` (a folder).
fn find_input(dir: &Path, stem: &str) -> PathBuf {
    let prefix = format!("{stem}.");
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .expect("case folder")
        .map(|e| e.expect("entry").path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            (p.is_file() && name.starts_with(&prefix)) || (p.is_dir() && name == stem)
        })
        .collect();
    assert_eq!(found.len(), 1, "{}: one {stem} input", dir.display());
    found.remove(0)
}

fn file_name(path: &Path) -> CString {
    c_string(path.file_name().and_then(|n| n.to_str()).expect("name"))
}

/// Checks one raw document: canonical as returned, `provenance.binding` is
/// `"c"`, and returns its conformance bytes.
fn checked(raw: &str) -> String {
    let value: Value = serde_json::from_str(raw).expect("JSON");
    assert_eq!(jcs::canonicalize(&value).expect("canonical"), raw);
    assert_eq!(value["provenance"]["binding"], "c");
    conformance(raw)
}

/// Every corpus case through the C ABI equals its `expected.json` without
/// `provenance`. File pairs go through both entry points; folder pairs
/// through `diffyman_diff_paths` only. A case without `expected.json` (not
/// blessed yet) is skipped here; the strict conformance run fails on it.
#[test]
fn corpus_matches_expected() {
    let mut cases = 0;
    let mut skipped = Vec::new();
    let mut groups: Vec<PathBuf> = fs::read_dir(corpus())
        .expect("tests/corpus")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_dir())
        .collect();
    groups.sort();
    for group in groups {
        let mut dirs: Vec<PathBuf> = fs::read_dir(&group)
            .expect("group")
            .map(|e| e.expect("entry").path())
            .filter(|p| p.join("options.json").is_file())
            .collect();
        dirs.sort();
        for dir in dirs {
            let name = dir
                .strip_prefix(group.parent().expect("corpus"))
                .expect("case path")
                .display()
                .to_string();
            let Ok(expected) = fs::read_to_string(dir.join("expected.json")) else {
                skipped.push(name);
                continue;
            };
            let expected = conformance(&expected);
            let options = c_string(&fs::read_to_string(dir.join("options.json")).expect("options"));
            let (left, right) = (find_input(&dir, "left"), find_input(&dir, "right"));

            let by_path = diff_paths(&left, &right, Some(&options), 0);
            assert_eq!(checked(by_path.document()), expected, "{name} (paths)");
            if !left.is_dir() && !right.is_dir() {
                let by_bytes = diff(
                    &fs::read(&left).expect("left"),
                    &fs::read(&right).expect("right"),
                    Some(&options),
                    Some(&file_name(&left)),
                    Some(&file_name(&right)),
                );
                assert_eq!(checked(by_bytes.document()), expected, "{name} (bytes)");
            }
            cases += 1;
        }
    }
    if !skipped.is_empty() {
        eprintln!("no expected.json yet, skipped: {}", skipped.join(", "));
    }
    assert!(cases >= 136, "only {cases} corpus cases compared");
}

#[test]
fn names_keep_their_final_component() {
    let out = diff(
        b"a\nb\n",
        b"a\nc\n",
        Some(c"{\"context\": 1}"),
        Some(c"left.txt"),
        Some(c"dir/right.txt"),
    );
    let doc: Value = serde_json::from_str(out.document()).unwrap();
    assert_eq!(doc["status"], "different");
    assert_eq!(doc["left"]["name"], "left.txt");
    assert_eq!(doc["right"]["name"], "right.txt");
    assert_eq!(doc["provenance"]["binding"], "c");
}

#[test]
fn null_and_blank_options_are_the_defaults() {
    let defaults = conformance(diff(b"x\n", b"y\n", Some(c"{}"), None, None).document());
    for options in [None, Some(c""), Some(c"  ")] {
        let out = diff(b"x\n", b"y\n", options, None, None);
        assert_eq!(conformance(out.document()), defaults, "{options:?}");
    }
}

#[test]
fn empty_inputs_may_be_null() {
    let (mut json, mut error) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: NULL buffers with length 0, valid out pointers.
    let status = unsafe {
        diffyman_diff(
            ptr::null(),
            0,
            ptr::null(),
            0,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            &raw mut json,
            &raw mut error,
        )
    };
    let out = Outcome {
        status,
        json: take(json),
        error: take(error),
    };
    let doc: Value = serde_json::from_str(out.document()).unwrap();
    assert_eq!(doc["status"], "identical");
}

#[test]
fn null_buffer_with_a_length_is_refused() {
    let (mut json, mut error) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: the NULL buffer is refused before it is read.
    let status = unsafe {
        diffyman_diff(
            ptr::null(),
            3,
            b"abc".as_ptr(),
            3,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            &raw mut json,
            &raw mut error,
        )
    };
    let out = Outcome {
        status,
        json: take(json),
        error: take(error),
    };
    assert_eq!(
        out.failure(DIFFYMAN_OTHER),
        "invalid argument: left is NULL but left_len is 3"
    );
}

#[test]
fn invalid_options_are_status_1() {
    for options in [c"{\"bogus\": true}", c"{", c"[]", c"{\"context\": -1}"] {
        let out = diff(b"", b"", Some(options), None, None);
        let message = out.failure(DIFFYMAN_INVALID_OPTIONS);
        assert!(message.starts_with("invalid options: "), "{message}");
    }
    let out = diff(b"", b"", Some(c"{\"context\": \xff}"), None, None);
    assert!(
        out.failure(DIFFYMAN_INVALID_OPTIONS)
            .starts_with("invalid options: options_json is not valid UTF-8")
    );
}

#[test]
fn names_must_be_utf8() {
    let out = diff(b"", b"", None, Some(c"l\xe9ft.txt"), None);
    assert!(
        out.failure(DIFFYMAN_OTHER)
            .starts_with("invalid argument: left_name is not valid UTF-8")
    );
}

/// Invalid UTF-8 under adapter `text` is a document with `status: "error"`,
/// not a failed call.
#[test]
fn engine_errors_inside_a_document_are_status_0() {
    let out = diff(
        b"\xff\n",
        b"a\n",
        Some(c"{\"adapter\": \"text\"}"),
        None,
        None,
    );
    let doc: Value = serde_json::from_str(out.document()).unwrap();
    assert_eq!(doc["status"], "error");
}

#[test]
fn missing_paths_are_status_2() {
    let missing = corpus().join("no-such-case/left.txt");
    let out = diff_paths(&missing, &missing, None, 0);
    assert!(out.failure(DIFFYMAN_IO).starts_with("cannot read "));
}

#[test]
fn read_limit_is_status_2() {
    let left = corpus().join("text/001-identical/left.txt");
    let out = diff_paths(&left, &left, None, 1);
    assert!(out.failure(DIFFYMAN_IO).starts_with("limit-exceeded: "));
    // 0 is the default rule, and a limit at the file size reads it.
    let size = fs::metadata(&left).unwrap().len();
    diff_paths(&left, &left, None, 0).document();
    diff_paths(&left, &left, None, size).document();
}

#[test]
fn paths_are_checked() {
    let (mut json, mut error) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: the NULL path is refused before anything is read.
    let status = unsafe {
        diffyman_diff_paths(
            ptr::null(),
            c"right.txt".as_ptr(),
            ptr::null(),
            0,
            &raw mut json,
            &raw mut error,
        )
    };
    let out = Outcome {
        status,
        json: take(json),
        error: take(error),
    };
    assert_eq!(
        out.failure(DIFFYMAN_OTHER),
        "invalid argument: left_path is NULL"
    );

    let not_utf8 = c"r\xe9ght.txt";
    let (mut json, mut error) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: valid pointers.
    let status = unsafe {
        diffyman_diff_paths(
            c"left.txt".as_ptr(),
            not_utf8.as_ptr(),
            ptr::null(),
            0,
            &raw mut json,
            &raw mut error,
        )
    };
    let out = Outcome {
        status,
        json: take(json),
        error: take(error),
    };
    assert!(
        out.failure(DIFFYMAN_OTHER)
            .starts_with("invalid argument: right_path is not valid UTF-8")
    );
}

#[test]
fn null_out_json_is_refused_and_reported() {
    let mut error = ptr::null_mut();
    // SAFETY: valid inputs; `out_json` is NULL on purpose.
    let status = unsafe {
        diffyman_diff(
            b"a".as_ptr(),
            1,
            b"b".as_ptr(),
            1,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
            &raw mut error,
        )
    };
    assert_eq!(status, DIFFYMAN_OTHER);
    assert_eq!(
        take(error).as_deref(),
        Some("invalid argument: out_json is NULL")
    );
}

#[test]
fn out_error_may_be_null() {
    let mut json: *mut c_char = ptr::null_mut();
    // SAFETY: valid inputs; `out_error` is NULL on purpose.
    let status = unsafe {
        diffyman_diff(
            b"a".as_ptr(),
            1,
            b"b".as_ptr(),
            1,
            c"{\"bogus\": 1}".as_ptr(),
            ptr::null(),
            ptr::null(),
            &raw mut json,
            ptr::null_mut(),
        )
    };
    assert_eq!(status, DIFFYMAN_INVALID_OPTIONS);
    assert!(json.is_null());
}

/// Both out pointers are reset on entry, so stale values never survive.
#[test]
fn out_pointers_are_reset() {
    let mut stale = *b"stale\0";
    let mut json: *mut c_char = stale.as_mut_ptr().cast();
    let mut error: *mut c_char = stale.as_mut_ptr().cast();
    // SAFETY: valid pointers; the stale values are overwritten, never read.
    let status = unsafe {
        diffyman_diff(
            b"a".as_ptr(),
            1,
            b"a".as_ptr(),
            1,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            &raw mut json,
            &raw mut error,
        )
    };
    assert_eq!(status, DIFFYMAN_OK);
    assert!(error.is_null());
    assert!(take(json).is_some());
}

#[test]
fn panics_are_caught() {
    let (mut json, mut error) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: valid out pointers.
    let status = unsafe {
        respond(
            &raw mut json,
            &raw mut error,
            || -> Result<String, Failure> { panic!("boom {}", 42) },
        )
    };
    assert_eq!(status, DIFFYMAN_OTHER);
    assert!(json.is_null());
    assert_eq!(take(error).as_deref(), Some("internal error: boom 42"));
}

#[test]
fn messages_never_lose_text_to_a_nul() {
    let message = message_string("cannot read a\0b".to_owned());
    assert_eq!(message.to_str().unwrap(), "cannot read a\u{FFFD}b");
}

#[test]
fn free_string_ignores_null() {
    // SAFETY: NULL is allowed.
    unsafe { diffyman_free_string(ptr::null_mut()) };
}

#[test]
fn versions_are_static_strings() {
    // SAFETY: static NUL-terminated strings.
    let (engine, schema) = unsafe {
        (
            CStr::from_ptr(diffyman_engine_version()),
            CStr::from_ptr(diffyman_schema_version()),
        )
    };
    assert_eq!(engine.to_str().unwrap(), diffyman::ENGINE_VERSION);
    assert_eq!(schema.to_str().unwrap(), diffyman::model::SCHEMA_VERSION);
    assert_eq!(schema.to_str().unwrap(), "0.4.0");
}

/// `include/diffyman.h` is what cbindgen generates from `src/lib.rs`. With
/// `DIFFYMAN_BLESS=1` the test writes it instead.
#[test]
fn header_is_current() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = cbindgen::Config::from_file(dir.join("cbindgen.toml")).expect("cbindgen.toml");
    let mut generated = Vec::new();
    cbindgen::Builder::new()
        .with_config(config)
        .with_src(dir.join("src/lib.rs"))
        .generate()
        .expect("cbindgen")
        .write(&mut generated);
    let path = dir.join("include/diffyman.h");
    if std::env::var_os("DIFFYMAN_BLESS").is_some() {
        fs::write(&path, &generated).expect("write include/diffyman.h");
        return;
    }
    let committed = fs::read(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "include/diffyman.h is out of date; regenerate it with \
         `DIFFYMAN_BLESS=1 cargo test -p diffyman-ffi header_is_current`"
    );
}
