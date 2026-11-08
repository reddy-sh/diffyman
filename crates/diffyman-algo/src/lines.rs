//! Line splitting (Diff Document spec §7.2).
//!
//! A line is a maximal run of characters that ends with a terminator (LF, CRLF
//! or a lone CR) or with the end of input. CR followed by LF is always one CRLF
//! terminator. A terminator at the very end of the input does not start a new
//! line, so `""` has 0 lines, `"a\n"` has 1 and `"a\r\n\r"` has 2.

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
}

/// Splits `text` into lines.
pub fn split_lines(text: &str) -> Vec<Line> {
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
                    terminator: Terminator::Lf,
                });
                i += 1;
                start = i;
            }
            b'\r' => {
                if bytes.get(i + 1) == Some(&b'\n') {
                    lines.push(Line {
                        start,
                        end: i,
                        terminator: Terminator::CrLf,
                    });
                    i += 2;
                } else {
                    lines.push(Line {
                        start,
                        end: i,
                        terminator: Terminator::Cr,
                    });
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        lines.push(Line {
            start,
            end: bytes.len(),
            terminator: Terminator::None,
        });
    }
    lines
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
}
