//! Renderers for diffyman Diff Documents (`docs/spec/diff-document.md`).
//!
//! Every renderer works from a finished document and never re-runs the diff.
//! Renderers that show file content also take the two inputs, because the
//! document holds line positions, not text. The inputs must be the bytes the
//! document was computed from: their size and SHA-256 are checked against the
//! file meta, and a mismatch is [`RenderError::InputMismatch`].
//!
//! - [`unified`]: a unified diff that `git apply` and `patch` accept.
//! - [`html()`]: one self-contained, accessible HTML report.
//! - [`summary`]: a one-line verdict plus stats, for terminals and logs.
//!
//! ```
//! use diffyman::Input;
//! use diffyman::model::Options;
//! use diffyman_render as render;
//!
//! let (left, right) = (b"a\nb\n".as_slice(), b"a\nc\n".as_slice());
//! let doc = diffyman::diff_document(
//!     Input::named(left, "x.txt"),
//!     Input::named(right, "x.txt"),
//!     &Options::default(),
//! )
//! .unwrap();
//! let patch = render::unified(&doc, left, right, &render::UnifiedOptions::default()).unwrap();
//! assert_eq!(patch, "--- a/x.txt\n+++ b/x.txt\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n");
//! assert_eq!(
//!     render::summary_line(&doc),
//!     "x.txt -> x.txt: different, 1 change in 1 hunk, +1 -1 lines, 50% similar"
//! );
//! ```
//!
//! Output depends only on the document (without `provenance`), the input
//! bytes and the options: no clock, locale or environment is read. Color for
//! [`unified`] is a plain flag; [`ColorChoice::resolve`] applies the
//! `NO_COLOR` convention for callers that write to a terminal.

mod html;
mod layout;
mod source;
mod summary;
mod text;
mod unified;

pub use html::{HtmlOptions, HtmlView, html};
pub use summary::{SummaryOptions, summary, summary_line};
pub use unified::{ColorChoice, UnifiedOptions, unified};

/// Quotes a path or name the way git does when it holds characters that
/// would break a line or a terminal (quotes, backslashes, tabs, control,
/// bidi-control and invisible characters): the name goes in double quotes
/// with C and octal escapes. Other names are returned unchanged. Every
/// renderer applies it to the names it prints.
pub fn quote_path(name: &str) -> std::borrow::Cow<'_, str> {
    text::quote_path(name)
}

/// Why a document could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenderError {
    /// The renderer does not handle this document kind (HTML supports `text` only;
    /// the unified and summary renderers also handle `binary`).
    #[error("documents of kind {0} cannot be rendered")]
    UnsupportedKind(String),
    /// The input bytes for this side are not the ones the document describes
    /// (size or SHA-256 differs).
    #[error("the {0} input does not match the document")]
    InputMismatch(&'static str),
    /// The document breaks a rule of the spec that rendering depends on, such
    /// as a hunk outside the input or ops that do not tile their hunk.
    #[error("invalid document: {0}")]
    InvalidDocument(String),
    /// The document has `status: "error"` and holds no diff to render as a
    /// patch. The value lists the error codes.
    #[error("the document has status error ({0})")]
    ErrorDocument(String),
}
