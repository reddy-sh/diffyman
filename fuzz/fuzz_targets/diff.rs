//! The whole engine on arbitrary `(options, left, right)` bytes.
//!
//! Input layout (see `diffyman_fuzz::chunks`): `[u16 len][options JSON]`,
//! `[u16 len][left bytes]`, then the right bytes. Asserts that the engine
//! never panics, that a rejected options text is rejected by the options
//! parser too, that every document passes `check_document`, that a second run
//! gives the same conformance bytes, and that the typed API agrees with the
//! JSON API.
#![no_main]

use diffyman::{Error, Input};
use diffyman_fuzz::{check_document, chunks, conformance_form};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [options, left, right] = chunks::<3>(data);
    let options = String::from_utf8_lossy(options);
    match diffyman::diff(left, right, &options) {
        Ok(json) => {
            let doc = check_document(&json, left, right);
            let again = diffyman::diff(left, right, &options).expect("second run succeeds");
            assert_eq!(
                conformance_form(&json),
                conformance_form(&again),
                "not deterministic"
            );
            let parsed = diffyman::parse_options(&options).expect("options parse");
            assert_eq!(
                parsed, doc.options,
                "document does not echo the resolved options"
            );
            let typed = diffyman::diff_document(Input::new(left), Input::new(right), &parsed)
                .expect("typed API accepts the same options");
            assert_eq!(
                diffyman::model::to_conformance_json(&typed).expect("typed document serializes"),
                conformance_form(&json),
                "typed and JSON APIs disagree"
            );
        }
        Err(Error::InvalidOptions(_)) => {
            assert!(
                diffyman::parse_options(&options).is_err(),
                "diff rejected options the parser accepts"
            );
        }
        Err(e) => panic!("engine failed: {e}"),
    }
});
