//! `diffyman-wasm`: the wasm-bindgen binding for browsers, Node.js and edge runtimes.
//!
//! JavaScript sees one main function:
//!
//! ```text
//! diff(left: Uint8Array, right: Uint8Array, optionsJson: string,
//!      leftName?: string, rightName?: string): string
//! ```
//!
//! It returns the canonical (RFC 8785) Diff Document as a string and throws an
//! `Error` when the options are invalid. Typed wrappers parse that string; no
//! diff logic lives on the JavaScript side. See `docs/adr/0007-binding-strategy.md`.
//!
//! The engine runs single-threaded and never touches the file system or the
//! clock on `wasm32-unknown-unknown` (`provenance.timing.totalMicros` is 0).

use diffyman::{Input, Settings};
use wasm_bindgen::prelude::*;

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
) -> Result<String, JsError> {
    diff_json(
        left,
        right,
        options_json,
        left_name.as_deref(),
        right_name.as_deref(),
    )
    .map_err(|message| JsError::new(&message))
}

/// The engine version, such as `0.0.1`.
#[wasm_bindgen(js_name = engineVersion)]
pub fn engine_version() -> String {
    diffyman::ENGINE_VERSION.to_owned()
}

/// The Diff Document schema version this build writes, such as `0.2.0`.
#[wasm_bindgen(js_name = schemaVersion)]
pub fn schema_version() -> String {
    diffyman::model::SCHEMA_VERSION.to_owned()
}

/// The platform-independent core of [`diff`], also used by native tests.
pub fn diff_json(
    left: &[u8],
    right: &[u8],
    options_json: &str,
    left_name: Option<&str>,
    right_name: Option<&str>,
) -> Result<String, String> {
    let settings = Settings {
        binding: "wasm".to_owned(),
    };
    diffyman::diff_inputs_with(
        input(left, left_name),
        input(right, right_name),
        options_json,
        &settings,
    )
    .map_err(|e| e.to_string())
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
        assert!(err.starts_with("invalid options"), "{err}");
    }

    #[test]
    fn versions() {
        assert_eq!(schema_version(), "0.2.0");
        assert!(!engine_version().is_empty());
    }
}
