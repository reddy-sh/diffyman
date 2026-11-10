//! `diffyman-ffi`: the stable C ABI of the diffyman diff engine.
//!
//! Five functions, declared in `include/diffyman.h` (generated from this file
//! by cbindgen; a test keeps it current):
//!
//! - `diffyman_diff`: two byte buffers, an options object as JSON text and
//!   optional names in; the canonical (RFC 8785) Diff Document out.
//! - `diffyman_diff_paths`: the same for two paths (files, folders or
//!   archives), read with the engine's bounded reader.
//! - `diffyman_free_string`: frees a string returned by the two above.
//! - `diffyman_engine_version`, `diffyman_schema_version`: static strings.
//!
//! The document is byte-identical to `diffyman diff --json` and to every
//! other binding once the top-level `provenance` key is removed;
//! `provenance.binding` is `"c"`. No diff logic lives here: each function
//! checks its arguments, calls the engine and maps its error to a status
//! code (docs/adr/0007-binding-strategy.md).
//!
//! No panic crosses the boundary: each call runs under `catch_unwind`, and a
//! panic is reported as `DIFFYMAN_OTHER` with an `internal error:` message.

#![allow(
    unsafe_code,
    reason = "a C ABI takes raw pointers; each unsafe block states what the caller guarantees"
)]

use std::ffi::{CStr, CString, c_char, c_int};
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::ptr;
use std::str::Utf8Error;

use diffyman::{Error, Input, Settings};

/// Success: `*out_json` holds the Diff Document and `*out_error` is NULL.
pub const DIFFYMAN_OK: c_int = 0;

/// The options are invalid: not UTF-8, not JSON, or an unknown key, a wrong
/// type or an unknown value (spec section 9). No document is produced; the
/// message starts with `invalid options:`.
pub const DIFFYMAN_INVALID_OPTIONS: c_int = 1;

/// An input path could not be read, or is over the read limit (the message
/// then starts with `limit-exceeded:`). `diffyman_diff_paths` only.
pub const DIFFYMAN_IO: c_int = 2;

/// Any other failure: a NULL or non-UTF-8 argument (`invalid argument:`), a
/// document that could not be serialized, or a panic inside the engine
/// (`internal error:`).
pub const DIFFYMAN_OTHER: c_int = 3;

/// `provenance.binding` of every document this library returns.
const BINDING: &str = "c";

/// Diffs two byte buffers and returns the canonical JSON Diff Document.
///
/// - `left`/`left_len`, `right`/`right_len`: the inputs. A pointer may be
///   NULL only when its length is 0 (an empty input).
/// - `options_json`: an options object as NUL-terminated UTF-8 JSON text
///   (spec section 9). NULL or blank text means `{}`, the defaults.
/// - `left_name`, `right_name`: NUL-terminated UTF-8 names, or NULL for no
///   name. Only the final path component is kept; names are part of the
///   document `id`.
/// - `out_json`: must not be NULL. On success it receives the document, a
///   NUL-terminated UTF-8 string the caller frees with
///   `diffyman_free_string`; otherwise NULL.
/// - `out_error`: may be NULL. On failure it receives the engine's message,
///   which the caller frees with `diffyman_free_string`; otherwise NULL.
///
/// Returns `DIFFYMAN_OK`, `DIFFYMAN_INVALID_OPTIONS` or `DIFFYMAN_OTHER`.
/// Inputs the engine can compare always give a document, even one with
/// `status: "error"` (for example invalid UTF-8 under adapter `text`).
/// Safe to call from several threads at once.
///
/// # Safety
///
/// Each non-NULL pointer must be valid for the access described above:
/// `left` and `right` readable for their lengths, the strings
/// NUL-terminated, and `out_json` and `out_error` writable. None of them is
/// kept after the call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn diffyman_diff(
    left: *const u8,
    left_len: usize,
    right: *const u8,
    right_len: usize,
    options_json: *const c_char,
    left_name: *const c_char,
    right_name: *const c_char,
    out_json: *mut *mut c_char,
    out_error: *mut *mut c_char,
) -> c_int {
    let run = || {
        // SAFETY: the caller guarantees these pointers as documented above.
        let (left, right, options, left_name, right_name) = unsafe {
            (
                bytes_arg(left, left_len, "left")?,
                bytes_arg(right, right_len, "right")?,
                options_arg(options_json)?,
                name_arg(left_name, "left_name")?,
                name_arg(right_name, "right_name")?,
            )
        };
        Ok(diffyman::diff_inputs_with(
            input(left, left_name),
            input(right, right_name),
            options,
            &settings(),
        )?)
    };
    // SAFETY: the caller guarantees `out_json` and `out_error`.
    unsafe { respond(out_json, out_error, run) }
}

/// Diffs two paths and returns the canonical JSON Diff Document.
///
/// Two files are read whole and diffed like `diffyman_diff`, named by their
/// final path components. A folder on either side gives a tree document
/// (folders and archives, spec section 19), as the CLI does.
///
/// - `left_path`, `right_path`: NUL-terminated UTF-8 paths; not NULL.
/// - `options_json`, `out_json`, `out_error`: as for `diffyman_diff`.
/// - `max_input_size`: the read limit in bytes for every input that is read
///   whole, like the CLI's `--max-input-size`. 0 keeps the default: a
///   regular file whole, a pipe or device up to 4 GiB, and a file paired
///   with a folder up to 4 GiB. An input over the limit is `DIFFYMAN_IO`
///   with a message that starts with `limit-exceeded:`.
///
/// Returns `DIFFYMAN_OK`, `DIFFYMAN_INVALID_OPTIONS`, `DIFFYMAN_IO` or
/// `DIFFYMAN_OTHER`. Safe to call from several threads at once.
///
/// # Safety
///
/// As for `diffyman_diff`: each non-NULL pointer must be valid, the strings
/// NUL-terminated, and `out_json` and `out_error` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn diffyman_diff_paths(
    left_path: *const c_char,
    right_path: *const c_char,
    options_json: *const c_char,
    max_input_size: u64,
    out_json: *mut *mut c_char,
    out_error: *mut *mut c_char,
) -> c_int {
    let run = || {
        // SAFETY: the caller guarantees these pointers as documented above.
        let (left, right, options) = unsafe {
            (
                path_arg(left_path, "left_path")?,
                path_arg(right_path, "right_path")?,
                options_arg(options_json)?,
            )
        };
        let options = diffyman::parse_options(options)?;
        let limit = (max_input_size != 0).then_some(max_input_size);
        let doc = diffyman::diff_paths_with_limit(left, right, &options, &settings(), limit)?;
        Ok(diffyman::model::to_canonical_json(&doc).map_err(Error::from)?)
    };
    // SAFETY: the caller guarantees `out_json` and `out_error`.
    unsafe { respond(out_json, out_error, run) }
}

/// Frees a string returned through `out_json` or `out_error`. NULL is
/// ignored.
///
/// # Safety
///
/// `s` must be NULL or a string this library returned that has not been
/// freed yet. Never pass it to `free()`, and never free the strings of
/// `diffyman_engine_version` or `diffyman_schema_version`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn diffyman_free_string(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: `s` came from `CString::into_raw` in `respond` (caller's
        // guarantee) and is freed once.
        drop(unsafe { CString::from_raw(s) });
    }
}

/// The engine version, such as `0.1.0`: a static NUL-terminated string that
/// must not be freed.
#[unsafe(no_mangle)]
pub extern "C" fn diffyman_engine_version() -> *const c_char {
    ENGINE_VERSION.as_ptr().cast()
}

/// The Diff Document schema version this library writes, such as `0.4.0`: a
/// static NUL-terminated string that must not be freed.
#[unsafe(no_mangle)]
pub extern "C" fn diffyman_schema_version() -> *const c_char {
    SCHEMA_VERSION.as_ptr().cast()
}

static ENGINE_VERSION: [u8; diffyman::ENGINE_VERSION.len() + 1] =
    nul_terminated(diffyman::ENGINE_VERSION);
static SCHEMA_VERSION: [u8; diffyman::model::SCHEMA_VERSION.len() + 1] =
    nul_terminated(diffyman::model::SCHEMA_VERSION);

/// `text` and a NUL byte, checked at compile time.
const fn nul_terminated<const N: usize>(text: &str) -> [u8; N] {
    let bytes = text.as_bytes();
    assert!(
        bytes.len() + 1 == N,
        "the array has room for the text and a NUL"
    );
    let mut out = [0; N];
    let mut i = 0;
    while i < bytes.len() {
        assert!(bytes[i] != 0, "a version has no NUL byte");
        out[i] = bytes[i];
        i += 1;
    }
    out
}

/// Why no document was produced: a status code and a message.
#[derive(Debug)]
struct Failure {
    status: c_int,
    message: String,
}

impl From<Error> for Failure {
    fn from(e: Error) -> Failure {
        let status = match e {
            Error::InvalidOptions(_) => DIFFYMAN_INVALID_OPTIONS,
            Error::Io(_) => DIFFYMAN_IO,
            // `Unsupported` needs a build without archive support; this
            // library always has it.
            Error::Canonical(_) | Error::Unsupported(_) => DIFFYMAN_OTHER,
        };
        Failure {
            status,
            message: e.to_string(),
        }
    }
}

fn invalid_argument(message: &str) -> Failure {
    Failure {
        status: DIFFYMAN_OTHER,
        message: format!("invalid argument: {message}"),
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

/// Clears both out pointers, runs `run` unless `out_json` is NULL, and
/// stores its document or its message. A panic in `run` is caught and
/// reported as `DIFFYMAN_OTHER`.
///
/// # Safety
///
/// `out_json` and `out_error` are each NULL or valid for a pointer write.
unsafe fn respond(
    out_json: *mut *mut c_char,
    out_error: *mut *mut c_char,
    run: impl FnOnce() -> Result<String, Failure>,
) -> c_int {
    // SAFETY: non-NULL out pointers are writable (caller's guarantee).
    unsafe {
        if !out_json.is_null() {
            out_json.write(ptr::null_mut());
        }
        if !out_error.is_null() {
            out_error.write(ptr::null_mut());
        }
    }
    let result = if out_json.is_null() {
        Err(invalid_argument("out_json is NULL"))
    } else {
        panic::catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|payload| {
            Err(Failure {
                status: DIFFYMAN_OTHER,
                message: format!("internal error: {}", panic_message(payload.as_ref())),
            })
        })
    };
    // Canonical JSON escapes every control character, so a NUL byte here
    // would be an engine bug.
    let result = result.and_then(|json| {
        CString::new(json).map_err(|_| Failure {
            status: DIFFYMAN_OTHER,
            message: "internal error: the document contains a NUL byte".to_owned(),
        })
    });
    match result {
        Ok(json) => {
            // SAFETY: `out_json` is not NULL here and is writable.
            unsafe { out_json.write(json.into_raw()) };
            DIFFYMAN_OK
        }
        Err(failure) => {
            if !out_error.is_null() {
                // SAFETY: `out_error` is not NULL here and is writable.
                unsafe { out_error.write(message_string(failure.message).into_raw()) };
            }
            failure.status
        }
    }
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

/// A message as a C string. A NUL byte (from a name or a path in the
/// message) is replaced by U+FFFD so the message is never cut short.
fn message_string(message: String) -> CString {
    CString::new(message).unwrap_or_else(|e| {
        let text = String::from_utf8_lossy(&e.into_vec()).replace('\0', "\u{FFFD}");
        CString::new(text).unwrap_or_default()
    })
}

/// A byte buffer. NULL is allowed only for an empty buffer.
///
/// # Safety
///
/// A non-NULL `ptr` is readable for `len` bytes for `'a`.
unsafe fn bytes_arg<'a>(ptr: *const u8, len: usize, what: &str) -> Result<&'a [u8], Failure> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err(invalid_argument(&format!(
            "{what} is NULL but {what}_len is {len}"
        )));
    }
    if isize::try_from(len).is_err() {
        return Err(invalid_argument(&format!("{what}_len is too large")));
    }
    // SAFETY: `ptr` is non-NULL and readable for `len <= isize::MAX` bytes.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// A NUL-terminated string as UTF-8, or `None` for NULL.
///
/// # Safety
///
/// A non-NULL `ptr` points to a NUL-terminated string that lives for `'a`.
unsafe fn c_str<'a>(ptr: *const c_char) -> Option<Result<&'a str, Utf8Error>> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: non-NULL and NUL-terminated (caller's guarantee).
    Some(unsafe { CStr::from_ptr(ptr) }.to_str())
}

/// The options text: NULL is `""` (the defaults). Text that is not UTF-8 is
/// invalid options, as text that is not JSON is.
///
/// # Safety
///
/// As for [`c_str`].
unsafe fn options_arg<'a>(ptr: *const c_char) -> Result<&'a str, Failure> {
    // SAFETY: forwarded from the caller.
    match unsafe { c_str(ptr) } {
        None => Ok(""),
        Some(Ok(text)) => Ok(text),
        Some(Err(e)) => Err(Failure {
            status: DIFFYMAN_INVALID_OPTIONS,
            message: format!("invalid options: options_json is not valid UTF-8 ({e})"),
        }),
    }
}

/// An optional input name.
///
/// # Safety
///
/// As for [`c_str`].
unsafe fn name_arg<'a>(ptr: *const c_char, what: &str) -> Result<Option<&'a str>, Failure> {
    // SAFETY: forwarded from the caller.
    match unsafe { c_str(ptr) } {
        None => Ok(None),
        Some(Ok(name)) => Ok(Some(name)),
        Some(Err(e)) => Err(invalid_argument(&format!(
            "{what} is not valid UTF-8 ({e})"
        ))),
    }
}

/// A required path.
///
/// # Safety
///
/// As for [`c_str`].
unsafe fn path_arg<'a>(ptr: *const c_char, what: &str) -> Result<&'a Path, Failure> {
    // SAFETY: forwarded from the caller.
    match unsafe { c_str(ptr) } {
        None => Err(invalid_argument(&format!("{what} is NULL"))),
        Some(Ok(path)) => Ok(Path::new(path)),
        Some(Err(e)) => Err(invalid_argument(&format!(
            "{what} is not valid UTF-8 ({e})"
        ))),
    }
}

#[cfg(test)]
mod tests;
