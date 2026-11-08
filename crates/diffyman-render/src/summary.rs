//! Plain-text summary: one verdict line plus stats.

use std::fmt::Write as _;

use diffyman_model::{DiffDocument, FileMeta, Kind, LineTerminator, MoveKind, Side, Status};

use crate::text;

/// Options for [`summary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryOptions {
    /// Add the indented stats lines after the verdict line.
    pub stats: bool,
}

impl Default for SummaryOptions {
    fn default() -> SummaryOptions {
        SummaryOptions { stats: true }
    }
}

/// The verdict line, without a trailing newline:
///
/// - `a.txt -> b.txt: different, 2 changes in 2 hunks, +2 -1 lines, 72.73% similar`
/// - `a.txt -> b.txt: identical` (with `, bytes differ only in ignored ways`
///   when the inputs are not byte-equal)
/// - `a.txt -> b.txt: error: invalid-utf8 (left)`
/// - `old -> new: different, 3 changed entries (1 added, 2 modified), 80% similar`
///   (kind `tree`; `partial` instead of `different` for a partial tree)
///
/// Names without a name in the document are `left` and `right`; names with
/// control or bidi characters are quoted like git does, so the line is safe
/// to print on a terminal.
pub fn summary_line(doc: &DiffDocument) -> String {
    let mut out = format!(
        "{} -> {}: ",
        name(&doc.left, "left"),
        name(&doc.right, "right")
    );
    match doc.status {
        Status::Error => {
            out.push_str("error: ");
            out.push_str(&warning_list(doc, true));
        }
        Status::Identical => {
            out.push_str("identical");
            if doc.left.sha256 != doc.right.sha256 {
                out.push_str(", bytes differ only in ignored ways");
            }
        }
        _ if doc.kind == Kind::Tree => {
            out.push_str(if doc.status == Status::Partial {
                "partial"
            } else {
                "different"
            });
            if let Some((s, t)) = doc
                .stats
                .as_ref()
                .and_then(|s| s.tree.as_ref().map(|t| (s, t)))
            {
                let changed: Vec<String> = [
                    (t.added, "added", "added"),
                    (t.removed, "removed", "removed"),
                    (t.modified, "modified", "modified"),
                    (t.moved, "moved", "moved"),
                    (t.type_changed, "type change", "type changes"),
                ]
                .iter()
                .filter(|(n, _, _)| *n > 0)
                .map(|(n, one, many)| {
                    if one == many {
                        format!("{n} {one}")
                    } else {
                        text::count(*n, one, many)
                    }
                })
                .collect();
                let _ = write!(
                    out,
                    ", {}",
                    text::count(s.change_count, "changed entry", "changed entries")
                );
                if !changed.is_empty() {
                    let _ = write!(out, " ({})", changed.join(", "));
                }
                let _ = write!(
                    out,
                    ", {} similar",
                    text::percent(s.similarity.basis_points())
                );
            }
        }
        _ => {
            out.push_str("different");
            if let Some(b) = doc.stats.as_ref().and_then(|s| s.bytes.as_ref()) {
                let s = doc.stats.as_ref().map_or(0, |s| s.change_count);
                let similarity = doc
                    .stats
                    .as_ref()
                    .map_or(0, |s| s.similarity.basis_points());
                let _ = write!(
                    out,
                    ", {}, +{} -{} bytes, {} similar",
                    text::count(s, "changed range", "changed ranges"),
                    b.added,
                    b.removed,
                    text::percent(similarity)
                );
            } else if let Some(s) = &doc.stats {
                let _ = write!(
                    out,
                    ", {} in {}, +{} -{} lines, {} similar",
                    text::count(s.change_count, "change", "changes"),
                    text::count(s.hunk_count, "hunk", "hunks"),
                    s.lines_added,
                    s.lines_removed,
                    text::percent(s.similarity.basis_points())
                );
            }
        }
    }
    out
}

/// The verdict line of [`summary_line`], then (with `stats`) one indented
/// line per topic. Every line ends with a newline.
///
/// ```text
/// left.txt -> right.txt: different, 2 changes in 2 hunks, +2 -1 lines, 72.73% similar
///   left:     5 lines, 36 bytes, LF
///   right:    6 lines, 42 bytes, LF, no final newline
///   lines:    2 added, 1 removed, 4 unchanged
///   warnings: no-newline-at-eof (right)
/// ```
///
/// A `moves:` line appears when the document was computed with
/// `detectMoves`. Lines that would say nothing (no warnings) are left out.
/// A tree document gets `left:` and `right:` lines with the side type
/// (`folder` or the archive's media type), entry count and size, and an
/// `entries:` line with the count of each entry status.
pub fn summary(doc: &DiffDocument, options: &SummaryOptions) -> String {
    let mut out = summary_line(doc);
    out.push('\n');
    if !options.stats {
        return out;
    }
    if doc.kind == Kind::Tree {
        let tree = doc.stats.as_ref().and_then(|s| s.tree.as_ref());
        let entries = |n: Option<u64>| n.map(|n| text::count(n, "entry", "entries"));
        for (label, meta, n) in [
            ("left", &doc.left, tree.map(|t| t.entries_left)),
            ("right", &doc.right, tree.map(|t| t.entries_right)),
        ] {
            let mut parts = vec![tree_side(meta)];
            parts.extend(entries(n));
            parts.push(text::count(meta.size, "byte", "bytes"));
            let _ = writeln!(out, "  {:<10}{}", format!("{label}:"), parts.join(", "));
        }
        if let Some(t) = tree {
            let _ = writeln!(
                out,
                "  entries:  {} added, {} removed, {} modified, {} moved, {}, {} unchanged",
                t.added,
                t.removed,
                t.modified,
                t.moved,
                text::count(t.type_changed, "type change", "type changes"),
                t.unchanged
            );
        }
        let warnings = warning_list(doc, false);
        if !warnings.is_empty() {
            let _ = writeln!(out, "  warnings: {warnings}");
        }
        return out;
    }
    let binary = doc.kind == Kind::Binary;
    let _ = writeln!(out, "  left:     {}", file_line(&doc.left, binary));
    let _ = writeln!(out, "  right:    {}", file_line(&doc.right, binary));
    if let Some(b) = doc.stats.as_ref().and_then(|s| s.bytes.as_ref()) {
        let _ = writeln!(
            out,
            "  bytes:    {} added, {} removed, {} unchanged",
            b.added, b.removed, b.unchanged
        );
    } else if let Some(s) = &doc.stats {
        let _ = writeln!(
            out,
            "  lines:    {} added, {} removed, {} unchanged",
            s.lines_added, s.lines_removed, s.lines_unchanged
        );
    }
    if doc.options.detect_moves && doc.status != Status::Error && doc.kind == Kind::Text {
        let pure = doc
            .moves
            .iter()
            .filter(|m| m.kind == MoveKind::Pure)
            .count();
        let edited = doc
            .moves
            .iter()
            .filter(|m| m.kind == MoveKind::Edited)
            .count();
        let lines: u64 = doc.moves.iter().map(|m| m.from.range.count).sum();
        let _ = writeln!(
            out,
            "  moves:    {} ({pure} pure, {edited} edited), {} moved",
            doc.moves.len(),
            text::count(lines, "line", "lines")
        );
    }
    let warnings = warning_list(doc, false);
    if !warnings.is_empty() {
        let _ = writeln!(out, "  warnings: {warnings}");
    }
    out
}

fn name<'a>(meta: &'a FileMeta, fallback: &'a str) -> std::borrow::Cow<'a, str> {
    text::quote_path(meta.name.as_deref().unwrap_or(fallback))
}

/// One side of a tree: `folder`, or the archive's media type.
fn tree_side(meta: &FileMeta) -> String {
    match meta.media_type.as_deref() {
        Some("inode/directory") => "folder".to_owned(),
        Some(media) => text::escape_controls(media).into_owned(),
        None => "unknown type".to_owned(),
    }
}

/// One input: lines (or, in a binary document, the media type), size,
/// terminators, BOM and final newline.
fn file_line(meta: &FileMeta, binary: bool) -> String {
    let mut parts = Vec::new();
    match (meta.line_count, &meta.media_type) {
        (Some(n), _) => parts.push(text::count(n, "line", "lines")),
        (None, Some(media)) if binary => parts.push(text::escape_controls(media).into_owned()),
        (None, _) => parts.push("not UTF-8".to_owned()),
    }
    parts.push(text::count(meta.size, "byte", "bytes"));
    if let Some(t) = meta.line_terminator {
        let style = match t {
            LineTerminator::Lf => "LF",
            LineTerminator::Crlf => "CRLF",
            LineTerminator::Cr => "CR",
            LineTerminator::Mixed => "mixed line endings",
            LineTerminator::None | LineTerminator::Unknown => "",
        };
        if !style.is_empty() {
            parts.push(style.to_owned());
        }
    }
    if meta.bom {
        parts.push("BOM".to_owned());
    }
    if meta.final_newline == Some(false) && meta.line_count.is_some_and(|n| n > 0) {
        parts.push("no final newline".to_owned());
    }
    parts.join(", ")
}

/// `code (side), code, ...` in document order; only errors when `errors_only`.
fn warning_list(doc: &DiffDocument, errors_only: bool) -> String {
    doc.warnings
        .iter()
        .filter(|w| !errors_only || w.severity == diffyman_model::Severity::Error)
        .map(|w| match w.side {
            Some(Side::Left) => format!("{} (left)", w.code.as_str()),
            Some(Side::Right) => format!("{} (right)", w.code.as_str()),
            _ => w.code.as_str().to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
