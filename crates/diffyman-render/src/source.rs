//! The input bytes behind a document: checked against the file meta, decoded
//! and split into lines exactly as the engine does (spec §7.1, §7.2).

use diffyman_model::lines::{IndexedText, TextInputs};
use diffyman_model::{DiffDocument, FileMeta, Kind, OpKind, Range, Status};

use crate::RenderError;

/// A line terminator.
pub(crate) use diffyman_model::lines::Terminator as Term;

/// The inputs a caller has just computed a document from, in the same
/// process: the bytes it passed to the engine and, for a text document, the
/// decoded inputs the engine gave back with it
/// (`diffyman::diff_document_with_text`).
///
/// The `*_computed` renderers trust that these are the document's inputs.
/// They check the sizes, the line counts and the document's structure (as
/// for any document), but do not hash, decode or split the inputs again,
/// which saves a SHA-256 of both inputs and a second line table. Use them
/// only for a document produced from exactly these inputs; for a document
/// read from elsewhere, use [`crate::unified`] and [`crate::html()`], which
/// check the inputs' SHA-256 digests.
#[derive(Debug, Clone, Copy)]
pub struct Computed<'a> {
    /// The left input's bytes.
    pub left: &'a [u8],
    /// The right input's bytes.
    pub right: &'a [u8],
    /// Both inputs as the engine decoded them, for a text document whose
    /// inputs decode. Without them the bytes are decoded and split here.
    pub text: Option<&'a TextInputs<'a>>,
}

/// The inputs a renderer was given.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Given<'a> {
    /// Bytes from anywhere: checked against the document's sizes and
    /// SHA-256 digests.
    Bytes(&'a [u8], &'a [u8]),
    /// The inputs the document was just computed from: sizes checked only.
    Computed(Computed<'a>),
}

impl<'a> Given<'a> {
    /// The raw bytes of both sides.
    pub fn bytes(&self) -> (&'a [u8], &'a [u8]) {
        match *self {
            Given::Bytes(l, r) => (l, r),
            Given::Computed(c) => (c.left, c.right),
        }
    }

    /// True when the SHA-256 digests are checked.
    fn hashed(&self) -> bool {
        matches!(self, Given::Bytes(..))
    }
}

/// One decoded side, split into lines like the engine does (spec §7.2: LF,
/// CRLF and a lone CR end a line, and a terminator at the very end does not
/// start a new line).
#[derive(Debug)]
pub(crate) struct SideText<'a> {
    /// The text after the BOM, with its lines.
    pub lines: IndexedText<'a>,
    /// The raw input started with a UTF-8 BOM.
    pub bom: bool,
}

impl<'a> SideText<'a> {
    /// Line `i` (0-based) without its terminator.
    pub fn content(&self, i: usize) -> &'a str {
        self.lines.content(i)
    }

    /// The terminator of line `i` (0-based).
    pub fn term(&self, i: usize) -> Term {
        self.lines.line(i).terminator
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }
}

/// Both sides; `None` for a side that does not decode (`invalid-utf8`).
#[derive(Debug)]
pub(crate) struct Source<'a> {
    pub left: Option<SideText<'a>>,
    pub right: Option<SideText<'a>>,
}

impl Source<'_> {
    /// Line counts of both sides (0 for a side that does not decode).
    pub fn counts(&self) -> (usize, usize) {
        (
            self.left.as_ref().map_or(0, SideText::len),
            self.right.as_ref().map_or(0, SideText::len),
        )
    }
}

/// Checks the inputs against the document and decodes them. For a document
/// whose status is not `error`, also checks that every hunk and op lies inside
/// the inputs and that the ops tile their hunks (spec §6.4), so the renderers
/// can index lines without further checks.
pub(crate) fn load<'a>(doc: &DiffDocument, given: Given<'a>) -> Result<Source<'a>, RenderError> {
    if doc.kind != Kind::Text {
        return Err(RenderError::UnsupportedKind(format!("{:?}", doc.kind)));
    }
    check_inputs(doc, given)?;
    let (left, right) = given.bytes();
    let decoded = match given {
        Given::Computed(Computed {
            text: Some(text), ..
        }) => [
            prepared(&doc.left, &text.left, left.len())?,
            prepared(&doc.right, &text.right, right.len())?,
        ],
        _ => [decode(&doc.left, left)?, decode(&doc.right, right)?],
    };
    let [left, right] = decoded;
    let source = Source { left, right };
    if doc.status != Status::Error {
        let (Some(l), Some(r)) = (&source.left, &source.right) else {
            return Err(invalid(
                "a side does not decode but the status is not error",
            ));
        };
        check_hunks(doc, l.len(), r.len())?;
    }
    Ok(source)
}

/// Checks that the inputs are the ones the document describes: their sizes,
/// and for inputs from anywhere their SHA-256 digests. Nothing is decoded.
pub(crate) fn check_inputs(doc: &DiffDocument, given: Given<'_>) -> Result<(), RenderError> {
    let (left, right) = given.bytes();
    for (meta, bytes, side) in [(&doc.left, left, "left"), (&doc.right, right, "right")] {
        if meta.size != bytes.len() as u64
            || (given.hashed() && meta.sha256 != diffyman_model::sha256_hex(bytes))
        {
            return Err(RenderError::InputMismatch(side));
        }
    }
    Ok(())
}

fn invalid(message: &str) -> RenderError {
    RenderError::InvalidDocument(message.to_owned())
}

/// A side the engine decoded: its text must fill the input (after the BOM
/// the file meta records) and have the recorded line count.
fn prepared<'a>(
    meta: &FileMeta,
    text: &IndexedText<'a>,
    size: usize,
) -> Result<Option<SideText<'a>>, RenderError> {
    let bom = meta.bom;
    if text.text().len() + if bom { 3 } else { 0 } != size {
        return Err(invalid("the decoded input does not match the document"));
    }
    if meta.line_count.is_some_and(|n| n != text.len() as u64) {
        return Err(invalid("lineCount does not match the input"));
    }
    Ok(Some(SideText {
        lines: text.clone(),
        bom,
    }))
}

fn decode<'a>(meta: &FileMeta, bytes: &'a [u8]) -> Result<Option<SideText<'a>>, RenderError> {
    let bom = bytes.starts_with(b"\xEF\xBB\xBF");
    let body = if bom { &bytes[3..] } else { bytes };
    let Ok(text) = std::str::from_utf8(body) else {
        return if meta.encoding.is_some() {
            Err(invalid(
                "the file meta says the input decodes, but it does not",
            ))
        } else {
            Ok(None)
        };
    };
    let lines = IndexedText::new(text);
    if meta.line_count.is_some_and(|n| n != lines.len() as u64) {
        return Err(invalid("lineCount does not match the input"));
    }
    Ok(Some(SideText { lines, bom }))
}

/// Converts a 1-based range to a 0-based `(start, count)` inside `n` lines.
pub(crate) fn range0(range: Range, n: usize) -> Option<(usize, usize)> {
    let start = usize::try_from(range.start).ok()?.checked_sub(1)?;
    let count = usize::try_from(range.count).ok()?;
    (start.checked_add(count)? <= n).then_some((start, count))
}

fn check_hunks(doc: &DiffDocument, nl: usize, nr: usize) -> Result<(), RenderError> {
    let (mut end_l, mut end_r) = (0usize, 0usize);
    for hunk in &doc.hunks {
        let (Some(hl), Some(hr)) = (range0(hunk.left, nl), range0(hunk.right, nr)) else {
            return Err(invalid("a hunk lies outside the input"));
        };
        if hl.0 < end_l || hr.0 < end_r {
            return Err(invalid("hunks overlap or are out of order"));
        }
        if hunk.ops.is_empty() {
            return Err(invalid("a hunk has no ops"));
        }
        let (mut at_l, mut at_r) = (hl.0, hr.0);
        for op in &hunk.ops {
            let (Some(ol), Some(or)) = (range0(op.left_range, nl), range0(op.right_range, nr))
            else {
                return Err(invalid("an op lies outside the input"));
            };
            if ol.0 != at_l || or.0 != at_r {
                return Err(invalid("the ops of a hunk do not tile it"));
            }
            let shape_ok = match op.op {
                OpKind::Equal => ol.1 == or.1 && ol.1 > 0,
                OpKind::Insert => ol.1 == 0 && or.1 > 0,
                OpKind::Delete => ol.1 > 0 && or.1 == 0,
                OpKind::Replace => ol.1 > 0 && or.1 > 0,
                OpKind::Ignored => ol.1 > 0 || or.1 > 0,
                OpKind::Unknown => false,
            };
            if !shape_ok {
                return Err(invalid(
                    "an op has an unknown kind or the wrong line counts",
                ));
            }
            at_l += ol.1;
            at_r += or.1;
        }
        if at_l != hl.0 + hl.1 || at_r != hr.0 + hr.1 {
            return Err(invalid("the ops of a hunk do not tile it"));
        }
        end_l = at_l;
        end_r = at_r;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(text: &str) -> Vec<(&str, Term)> {
        let side = SideText {
            lines: IndexedText::new(text),
            bom: false,
        };
        (0..side.len())
            .map(|i| (side.content(i), side.term(i)))
            .collect()
    }

    #[test]
    fn splits_like_the_spec() {
        assert!(shape("").is_empty());
        assert_eq!(shape("a"), [("a", Term::None)]);
        assert_eq!(shape("a\n"), [("a", Term::Lf)]);
        assert_eq!(shape("\n"), [("", Term::Lf)]);
        assert_eq!(shape("a\r\n\r"), [("a", Term::CrLf), ("", Term::Cr)]);
        assert_eq!(
            shape("x\ry\r\nz"),
            [("x", Term::Cr), ("y", Term::CrLf), ("z", Term::None)]
        );
    }

    #[test]
    fn ranges_stay_in_bounds() {
        assert_eq!(range0(Range { start: 1, count: 0 }, 0), Some((0, 0)));
        assert_eq!(range0(Range { start: 3, count: 2 }, 4), Some((2, 2)));
        assert_eq!(range0(Range { start: 4, count: 2 }, 4), None);
        assert_eq!(range0(Range { start: 0, count: 1 }, 4), None);
        assert_eq!(
            range0(
                Range {
                    start: u64::MAX,
                    count: u64::MAX
                },
                4
            ),
            None
        );
    }
}
