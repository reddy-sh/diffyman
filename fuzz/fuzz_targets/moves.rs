//! Moved-block detection (`text.moves@1`) through the engine with
//! `detectMoves: true`.
//!
//! Input layout: two option bytes, then `[u16 len][left]`, `[u16 len][extra]`
//! and move steps. When bit 0 of the second option byte is set, the right side is
//! the left side with blocks of lines moved by the steps (three bytes per move:
//! block start, block length, target) and `extra` inserted, so moves are
//! likely; otherwise the right side is `extra`. Asserts that every document
//! passes `check_document` (which checks the moves: ranges, kinds, ops they
//! reference, one move per line, and the inline spans of edited moves), that
//! a second run is identical, and that
//! moves never change hunks or stats (spec §6.5).
#![no_main]

use diffyman_fuzz::{check_document, chunks, conformance_form};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [a, b, rest @ ..] = data else {
        return;
    };
    let [left, extra, steps] = chunks::<3>(rest);
    let right = if b & 1 == 1 {
        rearrange(left, extra, steps)
    } else {
        extra.to_vec()
    };
    let options = options_json(*a, *b);
    let json = diffyman::diff(left, &right, &options).expect("generated options are valid");
    let doc = check_document(&json, left, &right);
    let again = diffyman::diff(left, &right, &options).expect("second run");
    assert_eq!(
        conformance_form(&json),
        conformance_form(&again),
        "not deterministic"
    );

    let without = options.replace(r#""detectMoves":true"#, r#""detectMoves":false"#);
    let plain = diffyman::diff(left, &right, &without).expect("options without moves");
    let plain = check_document(&plain, left, &right);
    assert_eq!(doc.stats, plain.stats, "moves changed stats");
    assert_eq!(doc.status, plain.status, "moves changed the status");
    assert_eq!(
        doc.hunks.len(),
        plain.hunks.len(),
        "moves changed the hunks"
    );
    for (h, p) in doc.hunks.iter().zip(&plain.hunks) {
        assert_eq!(
            (h.left, h.right),
            (p.left, p.right),
            "moves changed hunk {}",
            h.id
        );
        let shape = |hunk: &diffyman::model::Hunk| -> Vec<_> {
            hunk.ops
                .iter()
                .map(|o| (o.op, o.left_range, o.right_range))
                .collect()
        };
        assert_eq!(shape(h), shape(p), "moves changed the ops of {}", h.id);
    }
});

fn options_json(a: u8, b: u8) -> String {
    let algorithm = ["histogram", "myers", "patience", "histogram"][usize::from(a & 3)];
    let granularity = ["line", "word", "char", "smart"][usize::from((a >> 2) & 3)];
    let whitespace = ["none", "trailing", "change", "all"][usize::from((a >> 4) & 3)];
    let case = a & 0x40 != 0;
    let blank_lines = a & 0x80 != 0;
    let context = (b >> 1) & 7;
    let encoding = ["utf-8", "utf-16", "codepoint", "utf-8"][usize::from((b >> 4) & 3)];
    format!(
        concat!(
            r#"{{"algorithm":"{}","granularity":"{}","#,
            r#""ignore":{{"whitespace":"{}","case":{},"blankLines":{}}},"#,
            r#""context":{},"positionEncoding":"{}","detectMoves":true}}"#
        ),
        algorithm, granularity, whitespace, case, blank_lines, context, encoding
    )
}

/// The left lines with blocks moved per `steps` and `extra` inserted.
fn rearrange(left: &[u8], extra: &[u8], steps: &[u8]) -> Vec<u8> {
    let mut lines: Vec<&[u8]> = left.split_inclusive(|&c| c == b'\n').collect();
    for step in steps.as_chunks::<3>().0.iter().take(4) {
        if lines.is_empty() {
            break;
        }
        let start = usize::from(step[0]) % lines.len();
        let len = (usize::from(step[1]) % 8 + 1).min(lines.len() - start);
        let block: Vec<&[u8]> = lines.drain(start..start + len).collect();
        let at = usize::from(step[2]) % (lines.len() + 1);
        lines.splice(at..at, block);
    }
    let at = steps
        .first()
        .map_or(0, |&p| usize::from(p) % (lines.len() + 1));
    lines.insert(at, extra);
    lines.concat()
}
