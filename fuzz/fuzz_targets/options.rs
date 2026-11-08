//! Options JSON parsing (Diff Document spec §9) on arbitrary text.
//!
//! Asserts that parsing never panics and only fails with `InvalidOptions`;
//! that accepted options validate, serialize canonically and parse back to
//! the same value (from text and from a JSON value); and that a small diff
//! with them gives a document that passes `check_document` and echoes them.
//! This also drives `ignore.regex` compilation and its limits with arbitrary
//! patterns.
#![no_main]

use diffyman::Error;
use diffyman::model::jcs;
use diffyman_fuzz::check_document;
use libfuzzer_sys::fuzz_target;

const LEFT: &[u8] = b"id 7 start\n  alpha beta\n\nGamma 12:00\nend\n";
const RIGHT: &[u8] = b"id 123 start\nalpha  beta\n\n\ngamma 13:30\r\nend";

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let options = match diffyman::parse_options(&text) {
        Ok(options) => options,
        Err(Error::InvalidOptions(_)) => return,
        Err(e) => panic!("parse_options failed with a non-options error: {e}"),
    };
    diffyman::validate_options(&options).expect("parsed options validate");
    let canonical = jcs::to_canonical_string(&options).expect("options serialize");
    let back = diffyman::parse_options(&canonical).expect("serialized options parse");
    assert_eq!(back, options, "options do not round-trip through text");
    let value: serde_json::Value =
        serde_json::from_str(&canonical).expect("canonical text is JSON");
    let from_value = diffyman::parse_options_value(value).expect("options parse from a value");
    assert_eq!(from_value, options, "text and value parsing disagree");
    assert_eq!(
        jcs::to_canonical_string(&back).expect("options serialize"),
        canonical,
        "canonical options text is not stable"
    );

    let json = diffyman::diff(LEFT, RIGHT, &text).expect("accepted options give a document");
    let doc = check_document(&json, LEFT, RIGHT);
    assert_eq!(doc.options, options, "document does not echo the options");
});
