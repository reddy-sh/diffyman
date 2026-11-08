//! `diffyman-wasm`: the wasm-bindgen binding for browsers, Node.js and edge runtimes.
//!
//! JavaScript sees one main function:
//!
//! ```text
//! diff(left: Uint8Array, right: Uint8Array, optionsJson: string,
//!      leftName?: string, rightName?: string): string
//! ```
//!
//! It returns the canonical (RFC 8785) Diff Document as a string. When no
//! document can be produced it throws an `Error` whose `name` is
//! `DiffymanError` and whose `code` says why: `invalid-options`, or
//! `unsupported` for inputs this build cannot compare (a build without the
//! `archives` feature, given two archives; spec §19.7). Typed wrappers parse
//! the string; no diff logic lives on the JavaScript side. See
//! `docs/adr/0007-binding-strategy.md`.
//!
//! `features()` lists the optional features of the build (`archives`); see
//! `docs/adr/0008-wasm-target.md` for what each costs in size.
//!
//! The engine runs single-threaded and never touches the file system or the
//! clock on `wasm32-unknown-unknown` (`provenance.timing.totalMicros` is 0).

use diffyman::{Input, Settings};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    /// The JavaScript `Error` class.
    #[wasm_bindgen(js_name = Error)]
    type JsErrorObject;

    #[wasm_bindgen(constructor, js_class = "Error")]
    fn new(message: &str) -> JsErrorObject;

    #[wasm_bindgen(method, setter, js_class = "Error")]
    fn set_name(this: &JsErrorObject, name: &str);

    #[wasm_bindgen(method, setter, js_class = "Error")]
    fn set_code(this: &JsErrorObject, code: &str);
}

/// Why no document was produced: a stable code and a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffError {
    /// `invalid-options`, `unsupported`, or another code of
    /// `diffyman::Error::code`.
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

/// Diffs two byte arrays and returns the canonical JSON Diff Document.
///
/// `optionsJson` is an options object as JSON text; an empty string means `{}`.
/// `leftName` and `rightName` become `left.name` and `right.name` (they are part
/// of the document `id`); only the final path component is kept.
#[wasm_bindgen]
#[allow(
    clippy::needless_pass_by_value,
    reason = "wasm-bindgen cannot take Option<&str> parameters"
)]
pub fn diff(
    left: &[u8],
    right: &[u8],
    #[wasm_bindgen(js_name = optionsJson)] options_json: &str,
    #[wasm_bindgen(js_name = leftName)] left_name: Option<String>,
    #[wasm_bindgen(js_name = rightName)] right_name: Option<String>,
) -> Result<String, JsValue> {
    diff_json(
        left,
        right,
        options_json,
        left_name.as_deref(),
        right_name.as_deref(),
    )
    .map_err(|e| {
        let error = JsErrorObject::new(&e.message);
        error.set_name("DiffymanError");
        error.set_code(e.code);
        error.into()
    })
}

/// The engine version, such as `0.0.1`.
#[wasm_bindgen(js_name = engineVersion)]
pub fn engine_version() -> String {
    diffyman::ENGINE_VERSION.to_owned()
}

/// The Diff Document schema version this build writes, such as `0.4.0`.
#[wasm_bindgen(js_name = schemaVersion)]
pub fn schema_version() -> String {
    diffyman::model::SCHEMA_VERSION.to_owned()
}

/// The optional features of this build: `["archives"]` with archive
/// support, `[]` without.
#[wasm_bindgen]
pub fn features() -> Vec<String> {
    diffyman::FEATURES.iter().map(|f| (*f).to_owned()).collect()
}

/// The platform-independent core of [`diff`], also used by native tests.
pub fn diff_json(
    left: &[u8],
    right: &[u8],
    options_json: &str,
    left_name: Option<&str>,
    right_name: Option<&str>,
) -> Result<String, DiffError> {
    let settings = Settings {
        binding: "wasm".to_owned(),
    };
    diffyman::diff_inputs_with(
        input(left, left_name),
        input(right, right_name),
        options_json,
        &settings,
    )
    .map_err(DiffError::from)
}

fn input<'a>(bytes: &'a [u8], name: Option<&'a str>) -> Input<'a> {
    match name {
        Some(name) => Input::named(bytes, name),
        None => Input::new(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_canonical_json_with_wasm_provenance() {
        let json = diff_json(
            b"a\nb\n",
            b"a\nc\n",
            r#"{"context": 1}"#,
            Some("left.txt"),
            Some("dir/right.txt"),
        )
        .unwrap();
        assert!(json.contains(r#""status":"different""#));
        assert!(json.contains(r#""binding":"wasm""#));
        assert!(json.contains(r#""name":"right.txt""#));
    }

    #[test]
    fn invalid_options_are_an_error() {
        let err = diff_json(b"", b"", r#"{"bogus": true}"#, None, None).unwrap_err();
        assert_eq!(err.code, "invalid-options");
        assert!(err.message.starts_with("invalid options"), "{err:?}");
    }

    /// Two archives give a tree with the `archives` feature, and a typed
    /// `unsupported` error without it; text never depends on the feature.
    #[test]
    fn archives_follow_the_feature() {
        // An empty zip: the end-of-central-directory record alone.
        let mut zip = b"PK\x05\x06".to_vec();
        zip.extend_from_slice(&[0; 18]);
        let result = diff_json(&zip, &zip, "", None, None);
        if features().iter().any(|f| f == "archives") {
            assert!(result.unwrap().contains(r#""kind":"tree""#));
        } else {
            assert_eq!(result.unwrap_err().code, "unsupported");
        }
        assert!(diff_json(b"PK\x03\x04", b"x\n", "", None, None).is_ok());
    }

    #[test]
    fn versions() {
        assert_eq!(schema_version(), "0.4.0");
        assert!(!engine_version().is_empty());
    }
}
