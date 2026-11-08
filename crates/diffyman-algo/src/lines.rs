//! Line splitting (Diff Document spec §7.2).
//!
//! A line is a maximal run of characters that ends with a terminator (LF, CRLF
//! or a lone CR) or with the end of input. CR followed by LF is always one CRLF
//! terminator. A terminator at the very end of the input does not start a new
//! line, so `""` has 0 lines, `"a\n"` has 1 and `"a\r\n\r"` has 2.
//!
//! Three views of the same split:
//!
//! - [`split_lines`]: one [`Line`] per line (24 bytes each on 64-bit targets).
//! - [`LineIndex`]: only the line start offsets (4 bytes per line for inputs
//!   under 4 GiB), for large inputs. Line content and terminator are derived
//!   from the text on demand.
//! - [`LineCursor`]: a forward-only walk that stores nothing, for output that
//!   visits lines in order.

/// How a line ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Terminator {
    /// `\n`.
    Lf,
    /// `\r\n`.
    CrLf,
    /// A lone `\r`.
    Cr,
    /// End of input without a terminator.
    None,
}

impl Terminator {
    /// Length of the terminator in bytes.
    pub fn len(self) -> usize {
        match self {
            Terminator::Lf | Terminator::Cr => 1,
            Terminator::CrLf => 2,
            Terminator::None => 0,
        }
    }

    /// True for [`Terminator::None`].
    pub fn is_empty(self) -> bool {
        self == Terminator::None
    }

    /// The terminator text.
    pub fn as_str(self) -> &'static str {
        match self {
            Terminator::Lf => "\n",
            Terminator::CrLf => "\r\n",
            Terminator::Cr => "\r",
            Terminator::None => "",
        }
    }
}

/// One line of a text: byte offsets of its content and its terminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    /// Byte offset of the first content byte.
    pub start: usize,
    /// Byte offset just past the content (where the terminator starts).
    pub end: usize,
    /// How the line ends.
    pub terminator: Terminator,
}

impl Line {
    /// The line content (without its terminator).
    pub fn content<'a>(&self, text: &'a str) -> &'a str {
        &text[self.start..self.end]
    }

    /// Byte offset just past the terminator (where the next line starts).
    pub fn next_start(&self) -> usize {
        self.end + self.terminator.len()
    }
}

/// The line that starts at `start` (which must be below `bytes.len()`).
#[inline]
fn line_at(bytes: &[u8], start: usize) -> Line {
    match memchr::memchr2(b'\n', b'\r', &bytes[start..]) {
        Some(offset) => {
            let end = start + offset;
            let terminator = if bytes[end] == b'\n' {
                Terminator::Lf
            } else if bytes.get(end + 1) == Some(&b'\n') {
                Terminator::CrLf
            } else {
                Terminator::Cr
            };
            Line {
                start,
                end,
                terminator,
            }
        }
        None => Line {
            start,
            end: bytes.len(),
            terminator: Terminator::None,
        },
    }
}

/// Splits `text` into lines.
pub fn split_lines(text: &str) -> Vec<Line> {
    LineCursor::new(text).collect()
}

/// A forward-only walk over the lines of a text. Stores no per-line data.
#[derive(Debug, Clone)]
pub struct LineCursor<'a> {
    bytes: &'a [u8],
    /// Byte offset of the next line.
    pos: usize,
    /// 0-based number of the next line.
    line: usize,
}

impl<'a> LineCursor<'a> {
    /// A cursor before the first line of `text`.
    pub fn new(text: &'a str) -> LineCursor<'a> {
        LineCursor {
            bytes: text.as_bytes(),
            pos: 0,
            line: 0,
        }
    }

    /// 0-based number of the line [`Iterator::next`] returns next.
    pub fn position(&self) -> usize {
        self.line
    }

    /// Skips forward until the next line is `line` (0-based), or the text
    /// ends. A cursor never moves back: a smaller `line` does nothing.
    pub fn skip_to(&mut self, line: usize) {
        while self.line < line && self.next().is_some() {}
    }
}

impl Iterator for LineCursor<'_> {
    type Item = Line;

    fn next(&mut self) -> Option<Line> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let line = line_at(self.bytes, self.pos);
        self.pos = line.next_start();
        self.line += 1;
        Some(line)
    }
}

/// Line start offsets, narrow when the text is under 4 GiB.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Offsets {
    Narrow(Vec<u32>),
    Wide(Vec<usize>),
}

impl Offsets {
    #[inline]
    fn get(&self, i: usize) -> usize {
        match self {
            Offsets::Narrow(v) => v[i] as usize,
            Offsets::Wide(v) => v[i],
        }
    }

    fn len(&self) -> usize {
        match self {
            Offsets::Narrow(v) => v.len(),
            Offsets::Wide(v) => v.len(),
        }
    }
}

/// The lines of a text as start offsets only: 4 bytes per line for a text
/// under 4 GiB, 8 bytes otherwise. Gives the same lines as [`split_lines`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    /// Start of each line, then the text length (one more entry than lines).
    offsets: Offsets,
    seen_lf: bool,
    seen_crlf: bool,
    seen_cr: bool,
    last: Option<Terminator>,
}

impl LineIndex {
    /// Indexes the lines of `text`.
    pub fn new(text: &str) -> LineIndex {
        let bytes = text.as_bytes();
        // Every line but possibly the last ends with a terminator, and only a
        // lone CR ends a line without an LF: reserve for the LF count.
        let estimate = memchr::memchr_iter(b'\n', bytes).count() + 2;
        let mut index = LineIndex {
            offsets: Offsets::Narrow(Vec::new()),
            seen_lf: false,
            seen_crlf: false,
            seen_cr: false,
            last: None,
        };
        if u32::try_from(bytes.len()).is_ok() {
            let mut v: Vec<u32> = Vec::with_capacity(estimate);
            // Every offset is at most the text length, so it fits.
            index.scan(bytes, |offset| {
                v.push(u32::try_from(offset).unwrap_or(u32::MAX));
            });
            index.offsets = Offsets::Narrow(v);
        } else {
            let mut v: Vec<usize> = Vec::with_capacity(estimate);
            index.scan(bytes, |offset| v.push(offset));
            index.offsets = Offsets::Wide(v);
        }
        index
    }

    fn scan(&mut self, bytes: &[u8], mut push: impl FnMut(usize)) {
        push(0);
        let mut pos = 0;
        while pos < bytes.len() {
            let line = line_at(bytes, pos);
            match line.terminator {
                Terminator::Lf => self.seen_lf = true,
                Terminator::CrLf => self.seen_crlf = true,
                Terminator::Cr => self.seen_cr = true,
                Terminator::None => {}
            }
            self.last = Some(line.terminator);
            pos = line.next_start();
            push(pos);
        }
    }

    /// Number of lines.
    pub fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    /// True for a text with no lines (the empty text).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Byte offset where line `i` (0-based) starts; the text length for
    /// `i == len()`.
    #[inline]
    pub fn start(&self, i: usize) -> usize {
        self.offsets.get(i)
    }

    /// Byte range of line `i` (0-based) including its terminator.
    #[inline]
    pub fn raw_range(&self, i: usize) -> std::ops::Range<usize> {
        self.offsets.get(i)..self.offsets.get(i + 1)
    }

    /// Line `i` (0-based) of `text`, which must be the indexed text.
    #[inline]
    pub fn line(&self, text: &str, i: usize) -> Line {
        let bytes = text.as_bytes();
        let (start, next) = (self.offsets.get(i), self.offsets.get(i + 1));
        // Content never holds CR or LF, so the last bytes decide.
        let (end, terminator) = match bytes[next - 1] {
            b'\n' if next - 1 > start && bytes[next - 2] == b'\r' => (next - 2, Terminator::CrLf),
            b'\n' => (next - 1, Terminator::Lf),
            b'\r' => (next - 1, Terminator::Cr),
            _ => (next, Terminator::None),
        };
        Line {
            start,
            end,
            terminator,
        }
    }

    /// The distinct terminators used, in the order LF, CRLF, CR.
    pub fn terminators(&self) -> Vec<Terminator> {
        [
            (self.seen_lf, Terminator::Lf),
            (self.seen_crlf, Terminator::CrLf),
            (self.seen_cr, Terminator::Cr),
        ]
        .into_iter()
        .filter_map(|(seen, t)| seen.then_some(t))
        .collect()
    }

    /// The terminator of the last line, or `None` for the empty text.
    pub fn last_terminator(&self) -> Option<Terminator> {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(text: &str) -> Vec<(&str, Terminator)> {
        split_lines(text)
            .iter()
            .map(|l| (l.content(text), l.terminator))
            .collect()
    }

    #[test]
    fn spec_examples() {
        assert!(split_lines("").is_empty());
        assert_eq!(shape("a"), [("a", Terminator::None)]);
        assert_eq!(shape("a\n"), [("a", Terminator::Lf)]);
        assert_eq!(shape("\n"), [("", Terminator::Lf)]);
        assert_eq!(
            shape("a\r\n\r"),
            [("a", Terminator::CrLf), ("", Terminator::Cr)]
        );
    }

    #[test]
    fn mixed_terminators() {
        assert_eq!(
            shape("x\ry\r\nz\n\nw"),
            [
                ("x", Terminator::Cr),
                ("y", Terminator::CrLf),
                ("z", Terminator::Lf),
                ("", Terminator::Lf),
                ("w", Terminator::None),
            ]
        );
        assert_eq!(
            shape("\r\r\n"),
            [("", Terminator::Cr), ("", Terminator::CrLf)]
        );
    }

    #[test]
    fn multibyte_content() {
        assert_eq!(
            shape("é\n猫"),
            [("é", Terminator::Lf), ("猫", Terminator::None)]
        );
    }

    /// The index and the cursor give exactly the lines of `split_lines`.
    #[test]
    fn index_and_cursor_match_split_lines() {
        let texts = [
            "",
            "a",
            "a\n",
            "\n",
            "\r",
            "\r\n",
            "\n\r",
            "a\r\n\r",
            "x\ry\r\nz\n\nw",
            "\r\r\n",
            "\r\r\r\n\n\r",
            "é\n猫\r\r\n",
            "no newline at all, just one long line",
        ];
        for text in texts {
            let want = split_lines(text);
            let index = LineIndex::new(text);
            assert_eq!(index.len(), want.len(), "{text:?}");
            assert_eq!(index.is_empty(), want.is_empty());
            for (i, line) in want.iter().enumerate() {
                assert_eq!(index.line(text, i), *line, "{text:?} line {i}");
                assert_eq!(index.raw_range(i), line.start..line.next_start());
            }
            let mut seen: Vec<Terminator> = Vec::new();
            for t in [Terminator::Lf, Terminator::CrLf, Terminator::Cr] {
                if want.iter().any(|l| l.terminator == t) {
                    seen.push(t);
                }
            }
            assert_eq!(index.terminators(), seen, "{text:?}");
            assert_eq!(index.last_terminator(), want.last().map(|l| l.terminator));
            assert_eq!(LineCursor::new(text).collect::<Vec<_>>(), want);
        }
    }

    #[test]
    fn cursor_skips_forward_only() {
        let text = "a\nb\r\nc\rd";
        let mut cursor = LineCursor::new(text);
        cursor.skip_to(2);
        assert_eq!(cursor.position(), 2);
        assert_eq!(cursor.next().map(|l| l.content(text)), Some("c"));
        cursor.skip_to(1);
        assert_eq!(cursor.position(), 3);
        assert_eq!(cursor.next().map(|l| l.content(text)), Some("d"));
        cursor.skip_to(10);
        assert_eq!(cursor.position(), 4);
        assert_eq!(cursor.next(), None);
    }
}
