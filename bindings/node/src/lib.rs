//! `diffyman-node`: the napi-rs addon behind the `@diffyman/node` npm package.
//!
//! The native layer is deliberately thin (`docs/adr/0007-binding-strategy.md`):
//! bytes (or two paths) and an options JSON string go in, the canonical
//! (RFC 8785) Diff Document comes out as a string, with
//! `provenance.binding` set to `node`. Input coercion (strings, option
//! objects, `URL` paths), the `DiffymanError` class and the parsed-document
//! helpers live in `index.js`; no diff logic lives on the JavaScript side.
//!
//! JavaScript sees:
//!
//! ```text
//! diff(left: Uint8Array, right: Uint8Array, optionsJson: string,
//!      leftName?: string, rightName?: string): string
//! diffAsync(...same...): Promise<string>
//! diffPaths(left: string, right: string, optionsJson: string,
//!           maxInputSize?: number): string
//! diffPathsAsync(...same...): Promise<string>
//! engineVersion(), schemaVersion(), maxStreamBytes(), features()
//! ```
//!
//! The `Async` variants run the diff on the libuv thread pool (a napi
//! `AsyncTask`), so a large diff does not block the event loop. They copy
//! the input bytes before the work starts, so changing a buffer afterwards
//! cannot change the result.
//!
//! When no document can be produced, the call throws (or the promise
//! rejects with) an `Error` whose `code` is the engine's code
//! (`invalid-options`, `io`, `unsupported` or `canonical`) and whose message
//! is the engine's message. Everything else, such as invalid UTF-8 under
//! adapter `text`, is a document with `status: "error"`. A panic inside the
//! engine (a bug) never aborts the process: it is caught and thrown as a
//! plain `Error` with code `internal` and an `internal error:` message.

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use diffyman::{Input, Settings};
use napi::bindgen_prelude::AsyncTask;
use napi::{Env, JsError, Status, Task};
use napi_derive::napi;

/// The surface name written to `provenance.binding`.
const BINDING: &str = "node";

/// Why no document was produced: a stable code and the engine's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffError {
    /// `invalid-options`, `io`, `unsupported` or `canonical`
    /// (`diffyman::Error::code`).
    pub code: &'static str,
    /// The engine's message, such as `invalid options: ...`.
    pub message: String,
}

impl From<diffyman::Error> for DiffError {
    fn from(e: diffyman::Error) -> DiffError {
        DiffError {
            code: e.code(),
            message: e.to_string(),
        }
    }
}

fn settings() -> Settings {
    Settings {
        binding: BINDING.to_owned(),
    }
}

fn input<'a>(bytes: &'a [u8], name: Option<&'a str>) -> Input<'a> {
    match name {
        Some(name) => Input::named(bytes, name),
        None => Input::new(bytes),
    }
}

/// The code of a panic inside the engine: an engine bug, not one of the
/// engine's error codes, so `index.js` leaves it a plain `Error`.
pub const INTERNAL: &str = "internal";

/// Runs `run` and turns a panic into a [`DiffError`] with code
/// [`INTERNAL`] and an `internal error:` message. napi-rs calls a task's
/// `compute` from an `extern "C"` function, where a panic would abort the
/// whole Node.js process, so every diff runs under this guard (as in the C
/// ABI, the JNI and the Python binding).
fn guarded(run: impl FnOnce() -> Result<String, DiffError>) -> Result<String, DiffError> {
    panic::catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|payload| {
        Err(DiffError {
            code: INTERNAL,
            message: format!("internal error: {}", panic_message(payload.as_ref())),
        })
    })
}

/// The message of a panic payload (`panic!` with a literal or a format).
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s
    } else {
        "panic"
    }
}

/// Diffs two byte strings: the platform-independent core of [`diff`].
///
/// `options_json` is an options object as JSON text; blank text means `{}`.
/// Names become `left.name` and `right.name` (part of the document `id`);
/// only the final path component is kept.
pub fn diff_json(
    left: &[u8],
    right: &[u8],
    options_json: &str,
    left_name: Option<&str>,
    right_name: Option<&str>,
) -> Result<String, DiffError> {
    guarded(|| {
        diffyman::diff_inputs_with(
            input(left, left_name),
            input(right, right_name),
            options_json,
            &settings(),
        )
        .map_err(DiffError::from)
    })
}

/// Diffs two paths: the core of [`diff_paths`].
///
/// Two files are diffed like [`diff_json`], named by their final path
/// components; a folder on either side gives a tree document (spec §19).
/// Files are read with the engine's bounded reader, as the CLI does: `limit`
/// is the read limit in bytes of every input that is read whole (the CLI's
/// `--max-input-size`), and `None` keeps the default rule of
/// [`diffyman::read_path`] (a regular file whole, a pipe or device up to
/// [`diffyman::MAX_STREAM_BYTES`]). An input over the limit is an `io` error
/// whose message starts with `limit-exceeded:`.
pub fn diff_paths_json(
    left: &Path,
    right: &Path,
    options_json: &str,
    limit: Option<u64>,
) -> Result<String, DiffError> {
    guarded(|| {
        let options = diffyman::parse_options(options_json)?;
        let doc = diffyman::diff_paths_with_limit(left, right, &options, &settings(), limit)?;
        Ok(diffyman::model::to_canonical_json(&doc).map_err(diffyman::Error::from)?)
    })
}

/// A JavaScript `Error` with the engine's code and message.
fn js_error(env: Env, e: &DiffError) -> napi::Error {
    let error = napi::Error::new(e.code, e.message.clone());
    napi::Error::from(JsError::from(error).into_unknown(env))
}

/// The read limit from JavaScript: a non-negative integer (`index.js`
/// accepts a safe integer or a `bigint` and caps it at 2^53 − 1).
fn read_limit(limit: Option<i64>) -> napi::Result<Option<u64>> {
    limit
        .map(|n| {
            u64::try_from(n).map_err(|_| {
                napi::Error::new(
                    Status::InvalidArg,
                    "maxInputSize must be a non-negative integer",
                )
            })
        })
        .transpose()
}

/// Diffs two byte arrays and returns the canonical JSON Diff Document. Runs
/// on the calling thread; see [`diff_async`] for the thread-pool variant.
#[napi(catch_unwind)]
#[allow(
    clippy::needless_pass_by_value,
    reason = "napi-rs passes JavaScript strings as owned values"
)]
pub fn diff(
    env: Env,
    left: &[u8],
    right: &[u8],
    options_json: String,
    left_name: Option<String>,
    right_name: Option<String>,
) -> napi::Result<String> {
    diff_json(
        left,
        right,
        &options_json,
        left_name.as_deref(),
        right_name.as_deref(),
    )
    .map_err(|e| js_error(env, &e))
}

/// The work of [`diff_async`]: owned copies of the inputs.
pub struct DiffTask {
    left: Vec<u8>,
    right: Vec<u8>,
    options_json: String,
    left_name: Option<String>,
    right_name: Option<String>,
}

impl Task for DiffTask {
    type Output = Result<String, DiffError>;
    type JsValue = String;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        // Free the input copies as soon as the document exists.
        let (left, right) = (
            std::mem::take(&mut self.left),
            std::mem::take(&mut self.right),
        );
        Ok(diff_json(
            &left,
            &right,
            &self.options_json,
            self.left_name.as_deref(),
            self.right_name.as_deref(),
        ))
    }

    fn resolve(&mut self, env: Env, output: Self::Output) -> napi::Result<String> {
        output.map_err(|e| js_error(env, &e))
    }
}

/// Like [`diff`], on the libuv thread pool: returns a `Promise<string>`.
#[napi(catch_unwind)]
pub fn diff_async(
    left: &[u8],
    right: &[u8],
    options_json: String,
    left_name: Option<String>,
    right_name: Option<String>,
) -> AsyncTask<DiffTask> {
    AsyncTask::new(DiffTask {
        left: left.to_vec(),
        right: right.to_vec(),
        options_json,
        left_name,
        right_name,
    })
}

/// Diffs two paths (files, folders or archives) and returns the canonical
/// JSON Diff Document. Runs on the calling thread; see [`diff_paths_async`].
/// `max_input_size` is the read limit of [`diff_paths_json`].
#[napi(catch_unwind)]
#[allow(
    clippy::needless_pass_by_value,
    reason = "napi-rs passes JavaScript strings as owned values"
)]
pub fn diff_paths(
    env: Env,
    left: String,
    right: String,
    options_json: String,
    max_input_size: Option<i64>,
) -> napi::Result<String> {
    let limit = read_limit(max_input_size)?;
    diff_paths_json(Path::new(&left), Path::new(&right), &options_json, limit)
        .map_err(|e| js_error(env, &e))
}

/// The work of [`diff_paths_async`].
pub struct DiffPathsTask {
    left: PathBuf,
    right: PathBuf,
    options_json: String,
    limit: Option<u64>,
}

impl Task for DiffPathsTask {
    type Output = Result<String, DiffError>;
    type JsValue = String;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        Ok(diff_paths_json(
            &self.left,
            &self.right,
            &self.options_json,
            self.limit,
        ))
    }

    fn resolve(&mut self, env: Env, output: Self::Output) -> napi::Result<String> {
        output.map_err(|e| js_error(env, &e))
    }
}

/// Like [`diff_paths`], on the libuv thread pool (reading the files too):
/// returns a `Promise<string>`.
#[napi(catch_unwind)]
pub fn diff_paths_async(
    left: String,
    right: String,
    options_json: String,
    max_input_size: Option<i64>,
) -> napi::Result<AsyncTask<DiffPathsTask>> {
    let limit = read_limit(max_input_size)?;
    Ok(AsyncTask::new(DiffPathsTask {
        left: PathBuf::from(left),
        right: PathBuf::from(right),
        options_json,
        limit,
    }))
}

/// The engine version, such as `0.1.0`.
#[napi]
pub fn engine_version() -> String {
    diffyman::ENGINE_VERSION.to_owned()
}

/// The Diff Document schema version this build writes, such as `0.4.0`.
#[napi]
pub fn schema_version() -> String {
    diffyman::model::SCHEMA_VERSION.to_owned()
}

/// The default read limit for a path that is not a regular file (a pipe or
/// a device): 4 GiB.
#[napi]
pub fn max_stream_bytes() -> i64 {
    i64::try_from(diffyman::MAX_STREAM_BYTES).unwrap_or(i64::MAX)
}

/// The optional features of this build: `["archives", "fs"]`.
#[napi]
pub fn features() -> Vec<String> {
    diffyman::FEATURES.iter().map(|f| (*f).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_canonical_json_with_node_provenance() {
        let json = diff_json(
            b"a\nb\n",
            b"a\nc\n",
            r#"{"context": 1}"#,
            Some("left.txt"),
            Some("dir/right.txt"),
        )
        .unwrap();
        assert!(json.contains(r#""status":"different""#));
        assert!(json.contains(r#""binding":"node""#));
        assert!(json.contains(r#""name":"right.txt""#));
    }

    #[test]
    fn invalid_options_are_an_error() {
        let err = diff_json(b"", b"", r#"{"bogus": true}"#, None, None).unwrap_err();
        assert_eq!(err.code, "invalid-options");
        assert!(err.message.starts_with("invalid options"), "{err:?}");
    }

    #[test]
    fn paths_are_bounded_and_named() {
        let dir = std::env::temp_dir().join(format!("diffyman-node-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (old, new) = (dir.join("old.txt"), dir.join("new.txt"));
        std::fs::write(&old, b"a\nb\n").unwrap();
        std::fs::write(&new, b"a\nc\n").unwrap();
        let json = diff_paths_json(&old, &new, "", Some(4)).unwrap();
        assert!(json.contains(r#""name":"old.txt""#));
        let err = diff_paths_json(&old, &new, "", Some(3)).unwrap_err();
        assert_eq!(err.code, "io");
        assert!(err.message.starts_with("limit-exceeded: "), "{err:?}");
        let err = diff_paths_json(&old, &dir.join("missing"), "", None).unwrap_err();
        assert_eq!(err.code, "io");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn panics_are_caught() {
        // A panic in a diff, as in the thread-pool task's `compute`, must not
        // unwind into napi-rs's `extern "C"` callback (which aborts Node.js).
        let err = guarded(|| panic!("boom {}", 42)).unwrap_err();
        assert_eq!(err.code, INTERNAL);
        assert_eq!(err.message, "internal error: boom 42");
        let err = guarded(|| panic!("literal")).unwrap_err();
        assert_eq!(err.message, "internal error: literal");
        let err = guarded(|| panic::panic_any(7_u8)).unwrap_err();
        assert_eq!(err.message, "internal error: panic");
        assert_eq!(guarded(|| Ok("{}".to_owned())), Ok("{}".to_owned()));
    }

    #[test]
    fn versions_and_features() {
        assert_eq!(schema_version(), "0.4.0");
        assert_eq!(engine_version(), diffyman::ENGINE_VERSION);
        assert_eq!(features(), ["archives", "fs"]);
        assert_eq!(max_stream_bytes(), 1 << 32);
    }
}
