//! The input bytes behind a document: checked against the file meta, decoded
//! and split into lines exactly as the engine does (spec §7.1, §7.2).

use diffyman_model::{DiffDocument, FileMeta, Kind, OpKind, Range, Status};

use crate::RenderError;

/// A line terminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Term {
    /// End of input without a terminator.
    None,
    /// `\n`.
    Lf,
    /// `\r\n`.
    CrLf,
    /// A lone `\r`.
    Cr,
}

impl Term {
    /// The terminator's bytes.
    pub fn as_str(self) -> &'static str {
        match self {
            Term::None => "",
            Term::Lf => "\n",
            Term::CrLf => "\r\n",
            Term::Cr => "\r",
        }
    }
}

/// One line: byte offsets of its content in the decoded text, and its terminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Line {
    pub start: usize,
    pub end: usize,
    pub term: Term,
}

/// Splits text into lines (spec §7.2): LF, CRLF and a lone CR end a line, and
/// a terminator at the very end does not start a new line.
pub(crate) fn split_lines(text: &str) -> Vec<Line> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(Line {
                    start,
                    end: i,
                    term: Term::Lf,
                });
                i += 1;
                start = i;
            }
            b'\r' => {
                let crlf = bytes.get(i + 1) == Some(&b'\n');
                lines.push(Line {
                    start,
                    end: i,
                    term: if crlf { Term::CrLf } else { Term::Cr },
                });
                i += if crlf { 2 } else { 1 };
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        lines.push(Line {
            start,
            end: bytes.len(),
            term: Term::None,
        });
    }
    lines
}

/// One decoded side.
#[derive(Debug)]
pub(crate) struct SideText<'a> {
    /// The text after the BOM.
    pub text: &'a str,
    /// The raw input started with a UTF-8 BOM.
    pub bom: bool,
    pub lines: Vec<Line>,
}

impl SideText<'_> {
    /// Line `i` (0-based) without its terminator.
    pub fn content(&self, i: usize) -> &str {
        let l = self.lines[i];
        &self.text[l.start..l.end]
    }

    /// The terminator of line `i` (0-based).
    pub fn term(&self, i: usize) -> Term {
        self.lines[i].term
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
pub(crate) fn load<'a>(
    doc: &DiffDocument,
    left: &'a [u8],
    right: &'a [u8],
) -> Result<Source<'a>, RenderError> {
    if doc.kind != Kind::Text {
        return Err(RenderError::UnsupportedKind(format!("{:?}", doc.kind)));
    }
    let source = Source {
        left: decode(&doc.left, left, "left")?,
        right: decode(&doc.right, right, "right")?,
    };
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

/// Checks that the inputs are the ones the document describes (size and
/// SHA-256), without decoding them.
pub(crate) fn check_inputs(
    doc: &DiffDocument,
    left: &[u8],
    right: &[u8],
) -> Result<(), RenderError> {
    for (meta, bytes, side) in [(&doc.left, left, "left"), (&doc.right, right, "right")] {
        if meta.size != bytes.len() as u64 || meta.sha256 != diffyman_model::sha256_hex(bytes) {
            return Err(RenderError::InputMismatch(side));
        }
    }
    Ok(())
}

fn invalid(message: &str) -> RenderError {
    RenderError::InvalidDocument(message.to_owned())
}

fn decode<'a>(
    meta: &FileMeta,
    bytes: &'a [u8],
    side: &'static str,
) -> Result<Option<SideText<'a>>, RenderError> {
    if meta.size != bytes.len() as u64 || meta.sha256 != diffyman_model::sha256_hex(bytes) {
        return Err(RenderError::InputMismatch(side));
    }
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
    let lines = split_lines(text);
    if meta.line_count.is_some_and(|n| n != lines.len() as u64) {
        return Err(invalid("lineCount does not match the input"));
    }
    Ok(Some(SideText { text, bom, lines }))
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
        split_lines(text)
            .iter()
            .map(|l| (&text[l.start..l.end], l.term))
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
