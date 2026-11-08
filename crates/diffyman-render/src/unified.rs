//! Unified diff output (the format of `diff -u` and `git diff`).

use diffyman_model::{DiffDocument, Kind, OpKind, Severity, Status, WarningCode};

use crate::RenderError;
use crate::layout::{self, RHunk};
use crate::source::{self, SideText, Term};
use crate::text;

/// Options for [`unified`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnifiedOptions {
    /// Context lines around each change. `None` keeps the document's hunks
    /// (its `options.context`).
    pub context: Option<u32>,
    /// Add ANSI color (git's default colors). Resolve it with
    /// [`ColorChoice::resolve`] for terminal output.
    pub color: bool,
    /// Show control, bidi-control and invisible characters in content as
    /// `<LABEL>` markers. For terminal display only: the output is then no
    /// longer a byte-exact patch.
    pub escape_controls: bool,
    /// The `---` label. Default: `a/` plus the left name (`a/left` without one).
    pub left_label: Option<String>,
    /// The `+++` label. Default: `b/` plus the right name (`b/right` without one).
    pub right_label: Option<String>,
}

/// When to use color, following the `NO_COLOR` convention (<https://no-color.org>).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ColorChoice {
    /// Color when writing to a terminal and `NO_COLOR` is unset or empty.
    #[default]
    Auto,
    /// Always color, whatever `NO_COLOR` says (an explicit `--color=always`).
    Always,
    /// Never color.
    Never,
}

impl ColorChoice {
    /// Whether to color. `is_terminal` tells whether the output is a terminal,
    /// `no_color` is the value of the `NO_COLOR` environment variable, if set.
    pub fn resolve(self, is_terminal: bool, no_color: Option<&str>) -> bool {
        match self {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => is_terminal && no_color.is_none_or(str::is_empty),
        }
    }
}

const BOLD: &str = "\x1b[1m";
const CYAN: &str = "\x1b[36m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const RESET: &str = "\x1b[m";
const NO_NEWLINE: &str = "\\ No newline at end of file\n";

/// The codes of the error entries of `doc`.
fn error_codes(doc: &DiffDocument) -> Vec<&'static str> {
    doc.warnings
        .iter()
        .filter(|w| w.severity == Severity::Error)
        .map(|w| w.code.as_str())
        .collect()
}

/// Renders the document as a unified diff.
///
/// - Headers are `--- a/<left name>` and `+++ b/<right name>`, with names
///   quoted like git when they hold control or bidi characters. There is no
///   `diff --git` or `index` line and no timestamp.
/// - Hunk headers are `@@ -l,s +l,s @@`: a count of 1 is left out, and an
///   empty side gives the line before the insertion point with count 0.
/// - Lines keep their bytes: CRLF stays CRLF, and a line without a final
///   newline is followed by `\ No newline at end of file`. A line ending in a
///   lone CR is written with the CR and then LF, so each line stays on its own
///   output line (BSD `diff -u` does the same).
/// - A UTF-8 BOM is written at the start of line 1 of its side.
/// - `replace` and `ignored` ops are written as `-` lines, then `+` lines.
///   `equal` lines are written from the left input, so a patch made under an
///   ignore option applies to the left file.
/// - An identical document gives `""`. A binary document (kind `binary`), or
///   an error document whose inputs do not both decode as UTF-8, gives
///   `Binary files <left> and <right> differ` when their bytes differ; any
///   other error document is [`RenderError::ErrorDocument`].
pub fn unified(
    doc: &DiffDocument,
    left: &[u8],
    right: &[u8],
    options: &UnifiedOptions,
) -> Result<String, RenderError> {
    if doc.kind == Kind::Binary {
        source::check_inputs(doc, left, right)?;
    }
    let left_label = label(
        options.left_label.as_deref(),
        "a/",
        doc.left.name.as_deref(),
        "left",
    );
    let right_label = label(
        options.right_label.as_deref(),
        "b/",
        doc.right.name.as_deref(),
        "right",
    );
    if doc.kind == Kind::Binary {
        return match doc.status {
            Status::Error => Err(RenderError::ErrorDocument(error_codes(doc).join(", "))),
            Status::Identical => Ok(String::new()),
            _ => Ok(format!(
                "Binary files {left_label} and {right_label} differ\n"
            )),
        };
    }
    let src = source::load(doc, left, right)?;
    if doc.status == Status::Error {
        let errors: Vec<&str> = doc
            .warnings
            .iter()
            .filter(|w| w.severity == Severity::Error)
            .map(|w| w.code.as_str())
            .collect();
        if errors
            .iter()
            .all(|c| *c == WarningCode::InvalidUtf8.as_str())
            && !errors.is_empty()
        {
            if doc.left.sha256 == doc.right.sha256 {
                return Ok(String::new());
            }
            return Ok(format!(
                "Binary files {left_label} and {right_label} differ\n"
            ));
        }
        return Err(RenderError::ErrorDocument(errors.join(", ")));
    }
    let (Some(l), Some(r)) = (&src.left, &src.right) else {
        return Err(RenderError::InvalidDocument(
            "a side does not decode".to_owned(),
        ));
    };
    let (nl, nr) = src.counts();
    let hunks = layout::hunks(doc, nl, nr, options.context);
    if hunks.is_empty() {
        return Ok(String::new());
    }
    let mut w = Writer {
        out: String::new(),
        color: options.color,
        escape: options.escape_controls,
    };
    w.meta(&format!("--- {left_label}"));
    w.meta(&format!("+++ {right_label}"));
    for hunk in &hunks {
        w.hunk(hunk, l, r);
    }
    Ok(w.out)
}

fn label(given: Option<&str>, prefix: &str, name: Option<&str>, fallback: &str) -> String {
    let raw = match given {
        Some(g) => g.to_owned(),
        None => format!("{prefix}{}", name.unwrap_or(fallback)),
    };
    text::quote_path(&raw).into_owned()
}

/// `l,s` of a hunk header from a 0-based `(start, count)`.
pub(crate) fn header_range((start, count): (usize, usize)) -> String {
    match count {
        0 => format!("{start},0"),
        1 => format!("{}", start + 1),
        n => format!("{},{n}", start + 1),
    }
}

struct Writer {
    out: String,
    color: bool,
    escape: bool,
}

impl Writer {
    fn colored(&mut self, color: &str, s: &str) {
        if self.color {
            self.out.push_str(color);
            self.out.push_str(s);
            self.out.push_str(RESET);
        } else {
            self.out.push_str(s);
        }
    }

    fn meta(&mut self, line: &str) {
        self.colored(BOLD, line);
        self.out.push('\n');
    }

    fn hunk(&mut self, hunk: &RHunk<'_>, l: &SideText<'_>, r: &SideText<'_>) {
        let header = format!(
            "@@ -{} +{} @@",
            header_range(hunk.left()),
            header_range(hunk.right())
        );
        self.colored(CYAN, &header);
        self.out.push('\n');
        for op in &hunk.ops {
            if op.kind == OpKind::Equal {
                for i in op.l..op.l + op.ln {
                    self.line(' ', "", l, i);
                }
                continue;
            }
            for i in op.l..op.l + op.ln {
                self.line('-', RED, l, i);
            }
            for i in op.r..op.r + op.rn {
                self.line('+', GREEN, r, i);
            }
        }
    }

    fn line(&mut self, prefix: char, color: &str, side: &SideText<'_>, i: usize) {
        let bom = if i == 0 && side.bom { "\u{feff}" } else { "" };
        let content = side.content(i);
        let colored = self.color && !color.is_empty();
        if colored {
            self.out.push_str(color);
        }
        self.out.push(prefix);
        if self.escape {
            self.out.push_str(&text::escape_controls(bom));
            self.out.push_str(&text::escape_controls(content));
        } else {
            self.out.push_str(bom);
            self.out.push_str(content);
        }
        if colored {
            self.out.push_str(RESET);
        }
        match side.term(i) {
            Term::None => {
                self.out.push('\n');
                self.out.push_str(NO_NEWLINE);
            }
            Term::Cr => self.out.push_str("\r\n"),
            t => self.out.push_str(t.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_follows_no_color() {
        assert!(ColorChoice::Auto.resolve(true, None));
        assert!(ColorChoice::Auto.resolve(true, Some("")));
        assert!(!ColorChoice::Auto.resolve(true, Some("1")));
        assert!(!ColorChoice::Auto.resolve(false, None));
        assert!(ColorChoice::Always.resolve(false, Some("1")));
        assert!(!ColorChoice::Never.resolve(true, None));
    }

    #[test]
    fn header_ranges_follow_gnu() {
        assert_eq!(header_range((0, 0)), "0,0");
        assert_eq!(header_range((4, 0)), "4,0");
        assert_eq!(header_range((0, 1)), "1");
        assert_eq!(header_range((2, 3)), "3,3");
    }
}
