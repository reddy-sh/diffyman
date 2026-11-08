//! Line splitting and inline refinement (`text.refine@1`) with every
//! granularity and position encoding.
//!
//! Input layout: one selector byte (bits 0–1 whitespace mode, bit 2
//! `ignore.case`), then `[u16 len][left text]` and the right text (invalid
//! UTF-8 is replaced). Asserts:
//!
//! - `split_lines` gives exactly the spec §7.2 lines;
//! - `refine` is deterministic, and the limit decision does not depend on
//!   the position encoding;
//! - spans are in bounds, non-empty, sorted, non-overlapping and on
//!   character boundaries, and the UTF-16 and code point spans are the UTF-8
//!   spans converted;
//! - for `word` and `char`, the text outside the spans is the same on both
//!   sides (ignoring whitespace, after case folding when `ignore.case`), and
//!   identical sides get no spans.
#![no_main]

use diffyman::model::PositionEncoding;
use diffyman::split_lines;
use diffyman_algo::key::{WhitespaceMode, is_whitespace, simple_fold};
use diffyman_algo::refine::{
    self, Granularity, InlineSpan, RefineOptions, Refined, SideLine, SpanSide,
};
use diffyman_fuzz::{byte_offset, check_split, chunks};
use libfuzzer_sys::fuzz_target;

const ENCODINGS: [(refine::PositionEncoding, PositionEncoding); 3] = [
    (refine::PositionEncoding::Utf8, PositionEncoding::Utf8),
    (refine::PositionEncoding::Utf16, PositionEncoding::Utf16),
    (
        refine::PositionEncoding::Codepoint,
        PositionEncoding::Codepoint,
    ),
];

fuzz_target!(|data: &[u8]| {
    let Some((&selector, rest)) = data.split_first() else {
        return;
    };
    let [left, right] = chunks::<2>(rest);
    let (left, right) = (
        String::from_utf8_lossy(left),
        String::from_utf8_lossy(right),
    );
    let left_lines = split_lines(&left);
    let right_lines = split_lines(&right);
    check_split(&left, &left_lines);
    check_split(&right, &right_lines);
    let side = |text: &'_ str, lines: &[diffyman::Line]| -> Vec<(String, diffyman::Terminator)> {
        lines
            .iter()
            .map(|l| (l.content(text).to_owned(), l.terminator))
            .collect()
    };
    let l_owned = side(&left, &left_lines);
    let r_owned = side(&right, &right_lines);
    let l: Vec<SideLine<'_>> = l_owned
        .iter()
        .map(|(c, t)| SideLine {
            content: c,
            terminator: *t,
        })
        .collect();
    let r: Vec<SideLine<'_>> = r_owned
        .iter()
        .map(|(c, t)| SideLine {
            content: c,
            terminator: *t,
        })
        .collect();
    let whitespace = match selector & 3 {
        0 => WhitespaceMode::None,
        1 => WhitespaceMode::Trailing,
        2 => WhitespaceMode::Change,
        _ => WhitespaceMode::All,
    };
    let ignore_case = selector & 4 != 0;

    for granularity in [Granularity::Word, Granularity::Char, Granularity::Smart] {
        let mut utf8: Option<Refined> = None;
        for (encoding, model_encoding) in ENCODINGS {
            let options = RefineOptions {
                granularity,
                whitespace,
                ignore_case,
                position_encoding: encoding,
                masks: Vec::new(),
            };
            let result = refine::refine(&l, &r, &options);
            // The other encodings are compared with the UTF-8 result below.
            if encoding == refine::PositionEncoding::Utf8 {
                assert_eq!(
                    result,
                    refine::refine(&l, &r, &options),
                    "refine is not deterministic"
                );
            }
            let spans = match &result {
                Refined::SkippedLimit => {
                    if let Some(first) = &utf8 {
                        assert_eq!(
                            first,
                            &Refined::SkippedLimit,
                            "limit depends on the encoding"
                        );
                    }
                    utf8 = Some(result);
                    continue;
                }
                Refined::Spans(spans) => spans,
            };
            let bytes = to_bytes(spans, &l, &r, model_encoding);
            check_order(&bytes);
            match &utf8 {
                None => utf8 = Some(result.clone()),
                Some(Refined::Spans(first)) => {
                    assert_eq!(
                        &bytes, first,
                        "{encoding:?} spans are not the UTF-8 spans converted"
                    );
                }
                Some(Refined::SkippedLimit) => panic!("limit depends on the encoding"),
            }
            if l.is_empty() || r.is_empty() {
                // Only `replace` ops are refined; a side without lines gets nothing.
                assert!(bytes.is_empty(), "spans for an op with an empty side");
                continue;
            }
            if granularity != Granularity::Smart && encoding == refine::PositionEncoding::Utf8 {
                if l_owned == r_owned {
                    assert!(bytes.is_empty(), "identical sides got spans");
                }
                let rest_l = outside(&l, &bytes, SpanSide::Left, ignore_case);
                let rest_r = outside(&r, &bytes, SpanSide::Right, ignore_case);
                assert_eq!(
                    rest_l, rest_r,
                    "unchanged text differs ({granularity:?}, {whitespace:?})"
                );
            }
        }
    }
});

/// Spans with byte offsets; panics when an offset is out of bounds or off a
/// character boundary.
fn to_bytes(
    spans: &[InlineSpan],
    l: &[SideLine<'_>],
    r: &[SideLine<'_>],
    encoding: PositionEncoding,
) -> Vec<InlineSpan> {
    spans
        .iter()
        .map(|s| {
            let lines = if s.side == SpanSide::Left { l } else { r };
            let content = lines.get(s.line).expect("span line in bounds").content;
            let start = byte_offset(content, s.start, encoding)
                .unwrap_or_else(|| panic!("span {s:?} start off a boundary of {content:?}"));
            let end = byte_offset(content, s.end, encoding)
                .unwrap_or_else(|| panic!("span {s:?} end off a boundary of {content:?}"));
            InlineSpan {
                start: start as u64,
                end: end as u64,
                ..*s
            }
        })
        .collect()
}

fn check_order(spans: &[InlineSpan]) {
    for s in spans {
        assert!(s.start < s.end, "empty span {s:?}");
    }
    for pair in spans.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        assert!(
            (a.side, a.line, a.start) < (b.side, b.line, b.start),
            "spans out of order: {a:?} {b:?}"
        );
        if a.side == b.side && a.line == b.line {
            assert!(a.end <= b.start, "spans overlap: {a:?} {b:?}");
        }
    }
}

/// The text of one side outside its spans, without whitespace, folded when
/// `ignore_case`.
fn outside(
    lines: &[SideLine<'_>],
    spans: &[InlineSpan],
    side: SpanSide,
    ignore_case: bool,
) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        let mut pos = 0;
        let mut keep = |text: &str| {
            for c in text.chars().filter(|c| !is_whitespace(*c)) {
                out.push(if ignore_case { simple_fold(c) } else { c });
            }
        };
        for s in spans.iter().filter(|s| s.side == side && s.line == i) {
            let (start, end) = (s.start as usize, s.end as usize);
            keep(&line.content[pos..start]);
            pos = end;
        }
        keep(&line.content[pos..]);
    }
    out
}
