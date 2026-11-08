//! Self-contained HTML report: one file with inline CSS, no scripts and no
//! external resources. This module holds the page shell, the entry points
//! and the text view; `hex` and `tree` hold the binary and tree views.

use std::fmt::Write as _;

use diffyman_model::{
    Algorithm, DiffDocument, FileMeta, Granularity, Kind, LineTerminator, MoveKind, OpKind,
    PositionEncoding, Severity, Side, Span, Status, Warning, WhitespaceMode,
};

use crate::RenderError;
use crate::layout::{self, RHunk, ROp};
use crate::source::{self, Computed, SideText, Source, Term, range0};
use crate::text::{self, push_escaped};
use crate::unified::header_range;

/// Which views the report contains.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HtmlView {
    /// Side-by-side and inline, switched with a control that needs no script.
    #[default]
    Both,
    /// Side-by-side only.
    SideBySide,
    /// Inline (unified) only.
    Inline,
}

/// Options for [`html`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlOptions {
    /// Context lines around each change; `None` keeps the document's hunks.
    pub context: Option<u32>,
    /// Views to include.
    pub view: HtmlView,
    /// Page title. Default: `Diff: <left> → <right>`.
    pub title: Option<String>,
    /// Size cap for the report in bytes (default 8 MiB). Rows stop once the
    /// cap is reached and a notice says how many hunks are shown; the report
    /// may pass the cap by at most one row.
    pub max_bytes: usize,
    /// Longest line content shown, in bytes (default 10000). Longer lines are
    /// cut at a character boundary with a notice.
    pub max_line_bytes: usize,
    /// Bytes shown in hex dumps, both sides and every byte range together
    /// (default 65536). Each side of one range shows at most 256 bytes; once
    /// the total is used up, further ranges are listed without a dump and a
    /// notice says so.
    pub max_hex_bytes: usize,
    /// How many levels of child documents a tree report shows inline
    /// (default 4): 1 shows the changes inside the modified entries, 2 also
    /// those inside archives nested in the tree, and so on. 0 shows none.
    pub max_depth: u32,
    /// Entry content a tree report asks its [`EntryContents`] for, both sides
    /// and every entry together, in bytes (default 256 MiB). Entries past it
    /// are shown without their content.
    pub max_content_bytes: u64,
}

impl Default for HtmlOptions {
    fn default() -> HtmlOptions {
        HtmlOptions {
            context: None,
            view: HtmlView::Both,
            title: None,
            max_bytes: 8 * 1024 * 1024,
            max_line_bytes: 10_000,
            max_hex_bytes: 64 * 1024,
            max_depth: 4,
            max_content_bytes: 256 << 20,
        }
    }
}

/// Supplies the contents of the modified entries of a tree document, so the
/// HTML report can show the changes inside them, not only their positions.
///
/// It is implemented for every `FnMut(&[&str]) -> Option<(Vec<u8>, Vec<u8>)>`.
pub trait EntryContents {
    /// The left and right content of the `modified` file entry at `path`:
    /// the entry's `path` in each enclosing tree, outermost first, so
    /// `["inner.zip", "a.txt"]` is `a.txt` inside the archive entry
    /// `inner.zip`. `None` when the content is not available.
    ///
    /// Each content must be the bytes the child document describes: its size
    /// and SHA-256 are checked, and an entry whose content does not match is
    /// shown without it.
    fn contents(&mut self, path: &[&str]) -> Option<(Vec<u8>, Vec<u8>)>;
}

impl<F> EntryContents for F
where
    F: FnMut(&[&str]) -> Option<(Vec<u8>, Vec<u8>)>,
{
    fn contents(&mut self, path: &[&str]) -> Option<(Vec<u8>, Vec<u8>)> {
        self(path)
    }
}

/// An [`EntryContents`] that has no content.
struct NoContents;

impl EntryContents for NoContents {
    fn contents(&mut self, _path: &[&str]) -> Option<(Vec<u8>, Vec<u8>)> {
        None
    }
}

/// Bytes kept free for the page header (styles, file details, warnings and
/// the moves list) when the size cap is split between the views.
pub(crate) const HEADER_RESERVE: usize = 96 * 1024;
/// A row of the file details table: label and value for one side.
type MetaRow = (&'static str, fn(&FileMeta) -> String);
/// Entries listed in the moved-blocks section.
const MAX_LISTED_MOVES: usize = 200;

/// Renders the document as one self-contained HTML page.
///
/// - Semantic markup: a `<table>` per view with column headers, one
///   `<tbody>` per hunk, `<del>` and `<ins>` for inline changes.
/// - Changes are marked with `−`, `+`, `⇄` (moved) and `≈` (ignored) and
///   with hidden text for screen readers, not only with color. The palette
///   (orange for removed, blue for added, purple for moved) is safe for the
///   common forms of color blindness, and follows `prefers-color-scheme`.
/// - Every piece of content and every name is escaped. Control, invisible and
///   bidi-control characters are replaced by visible labels such as `RLO`, so
///   they cannot reorder the page; a notice says when there are any.
/// - No script and no external resource. The view switch is CSS only.
///
/// By kind:
///
/// - `text`: side-by-side and inline views of the hunks ([`HtmlView`]).
/// - `binary`: a table of the changed byte ranges (offset, length, change)
///   with a hex dump of both sides of each range, within
///   [`HtmlOptions::max_hex_bytes`].
/// - `tree`: the changed entries (status, path, both sides' type, size and
///   SHA-256), the unchanged ones in a collapsed list, the tree warnings
///   flagged at the top, and each modified entry's child document inline
///   in a collapsed `<details>`, down to [`HtmlOptions::max_depth`]. The
///   inputs are not read: pass empty slices. Child documents are shown
///   from the document alone (line and byte positions, no content); use
///   [`html_with_contents`] to show their content.
///
/// For kinds `text` and `binary`, `left` and `right` must be the inputs the
/// document describes.
pub fn html(
    doc: &DiffDocument,
    left: &[u8],
    right: &[u8],
    options: &HtmlOptions,
) -> Result<String, RenderError> {
    html_with_contents(doc, left, right, &mut NoContents, options)
}

/// [`html`], with the contents of tree entries from `contents`, so the
/// child documents of modified entries show the changed lines and bytes
/// themselves. Contents are asked for in path order, only for entries the
/// report shows, and within [`HtmlOptions::max_content_bytes`]. For kinds
/// other than `tree`, `contents` is not used.
pub fn html_with_contents(
    doc: &DiffDocument,
    left: &[u8],
    right: &[u8],
    contents: &mut dyn EntryContents,
    options: &HtmlOptions,
) -> Result<String, RenderError> {
    page(doc, source::Given::Bytes(left, right), contents, options)
}

/// [`html`] for a document the caller has just computed from `inputs`: the
/// same page, without hashing, decoding and splitting the inputs again (see
/// [`Computed`]). A tree document's entries have no content here; use
/// [`html_with_contents`] for that.
pub fn html_computed(
    doc: &DiffDocument,
    inputs: &Computed<'_>,
    options: &HtmlOptions,
) -> Result<String, RenderError> {
    page(
        doc,
        source::Given::Computed(*inputs),
        &mut NoContents,
        options,
    )
}

fn page(
    doc: &DiffDocument,
    given: source::Given<'_>,
    contents: &mut dyn EntryContents,
    options: &HtmlOptions,
) -> Result<String, RenderError> {
    match doc.kind {
        Kind::Text => text_page(doc, given, options),
        Kind::Binary => crate::hex::page(doc, given, options),
        Kind::Tree => crate::tree::page(doc, contents, options),
        other => Err(RenderError::UnsupportedKind(format!("{other:?}"))),
    }
}

/// The page around a view: head, styles, title, `header` (written by the
/// caller, after the title so it sees the title's characters in `flags`),
/// `main` and the footer.
pub(crate) fn assemble(
    doc: &DiffDocument,
    options: &HtmlOptions,
    extra_css: &str,
    flags: &mut Flags,
    main: &str,
    header: impl FnOnce(&mut String, &mut Flags),
) -> String {
    let mut out = String::with_capacity(main.len() + 16_384);
    let title = match &options.title {
        Some(t) => t.clone(),
        None => format!(
            "Diff: {} → {}",
            doc.left.name.as_deref().unwrap_or("left"),
            doc.right.name.as_deref().unwrap_or("right")
        ),
    };
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<meta name=\"color-scheme\" content=\"light dark\">\n");
    out.push_str("<meta name=\"generator\" content=\"diffyman\">\n<title>");
    push_escaped(&mut out, &text::escape_controls(&title));
    out.push_str("</title>\n<style>\n");
    out.push_str(CSS);
    out.push_str(extra_css);
    out.push_str("</style>\n</head>\n<body>\n<header>\n<h1>");
    plain(&mut out, &title, flags);
    out.push_str("</h1>\n");
    header(&mut out, flags);
    out.push_str("</header>\n<main>\n");
    out.push_str(main);
    out.push_str("</main>\n<footer>\n<p>Diff Document <code>");
    push_escaped(&mut out, &doc.id);
    out.push_str("</code>, schema ");
    push_escaped(&mut out, &doc.schema_version);
    out.push_str(".</p>\n</footer>\n</body>\n</html>\n");
    out
}

/// `left → right`, the first line of every header.
pub(crate) fn files_line(out: &mut String, doc: &DiffDocument, flags: &mut Flags) {
    out.push_str("<p class=\"files\">");
    plain(out, doc.left.name.as_deref().unwrap_or("left"), flags);
    out.push_str(" <span aria-hidden=\"true\">→</span><span class=\"sr\">compared with</span> ");
    plain(out, doc.right.name.as_deref().unwrap_or("right"), flags);
    out.push_str("</p>\n");
}

/// The status line of an error document.
pub(crate) const ERROR_STATUS: &str = "<p class=\"status st-error\"><strong>Error.</strong> The comparison could not be completed; see the warnings below.</p>\n";

/// The notice about bidi-control or invisible characters, when there were any.
pub(crate) fn char_notices(out: &mut String, flags: &Flags) {
    if flags.bidi {
        out.push_str("<p class=\"notice\" role=\"note\"><strong>Bidirectional control characters.</strong> The content holds characters that can reorder how text is displayed (as in Trojan Source attacks). They are shown as labels such as <span class=\"inv\">RLO</span> and have no effect here.</p>\n");
    } else if flags.invisible {
        out.push_str("<p class=\"notice\" role=\"note\"><strong>Invisible characters.</strong> The content holds control or zero-width characters. They are shown as labels such as <span class=\"inv\">ZWSP</span>.</p>\n");
    }
}

/// The document's warnings as a section with a heading, when there are any.
pub(crate) fn warnings_section(out: &mut String, warnings: &[Warning]) {
    if warnings.is_empty() {
        return;
    }
    out.push_str("<section aria-labelledby=\"warnings-h\">\n<h2 id=\"warnings-h\">Warnings</h2>\n");
    warnings_list(out, warnings);
    out.push_str("</section>\n");
}

/// The warnings as a list, one item per warning.
pub(crate) fn warnings_list(out: &mut String, warnings: &[Warning]) {
    out.push_str("<ul class=\"warnings\">\n");
    for w in warnings {
        let (class, label) = if w.severity == Severity::Error {
            ("sev-error", "Error")
        } else {
            ("sev-warning", "Warning")
        };
        let _ = write!(out, "<li class=\"{class}\"><strong>{label}</strong> <code>");
        push_escaped(out, w.code.as_str());
        out.push_str("</code>");
        match w.side {
            Some(Side::Left) => out.push_str(" (left)"),
            Some(Side::Right) => out.push_str(" (right)"),
            _ => {}
        }
        out.push_str(": ");
        push_escaped(out, &text::escape_controls(&w.message));
        out.push_str("</li>\n");
    }
    out.push_str("</ul>\n");
}

/// Whether changed lines show their terminators: the sides use different
/// styles, or one mixes them.
fn show_eol(doc: &DiffDocument) -> bool {
    doc.left.line_terminator != doc.right.line_terminator
        || [doc.left.line_terminator, doc.right.line_terminator]
            .contains(&Some(LineTerminator::Mixed))
}

/// A text document's hunks as one inline table, for a tree report: element
/// ids start with `ids`, and the table stops near `budget` bytes. Returns
/// the HTML (empty when there is nothing to show) and whether it was cut.
pub(crate) fn text_fragment(
    doc: &DiffDocument,
    left: &[u8],
    right: &[u8],
    ids: &str,
    options: &HtmlOptions,
    budget: usize,
    flags: &mut Flags,
) -> Result<(String, bool), RenderError> {
    let src = source::load(doc, source::Given::Bytes(left, right))?;
    if doc.status == Status::Error {
        return Ok((String::new(), false));
    }
    let (nl, nr) = src.counts();
    let hunks = layout::hunks(doc, nl, nr, options.context);
    if hunks.is_empty() {
        return Ok((String::new(), false));
    }
    let ctx = Ctx {
        doc,
        src: &src,
        hunks: &hunks,
        moves: MoveIndex::new(doc, nl, nr),
        show_eol: show_eol(doc),
        max_line: options.max_line_bytes,
        encoding: doc.position_encoding,
        digits: nl.max(nr).max(1).to_string().len(),
        first_view: ViewKind::Inline,
        ids,
        embedded: true,
    };
    Ok(ctx.view(ViewKind::Inline, budget, flags))
}

fn text_page(
    doc: &DiffDocument,
    given: source::Given<'_>,
    options: &HtmlOptions,
) -> Result<String, RenderError> {
    let src = source::load(doc, given)?;
    let (nl, nr) = src.counts();
    let hunks = if doc.status == Status::Error {
        Vec::new()
    } else {
        layout::hunks(doc, nl, nr, options.context)
    };
    let views: &[ViewKind] = match options.view {
        HtmlView::Both => &[ViewKind::Split, ViewKind::Inline],
        HtmlView::SideBySide => &[ViewKind::Split],
        HtmlView::Inline => &[ViewKind::Inline],
    };
    let ctx = Ctx {
        doc,
        src: &src,
        hunks: &hunks,
        moves: MoveIndex::new(doc, nl, nr),
        show_eol: show_eol(doc),
        max_line: options.max_line_bytes,
        encoding: doc.position_encoding,
        digits: nl.max(nr).max(1).to_string().len(),
        first_view: views[0],
        ids: "",
        embedded: false,
    };
    let mut flags = Flags::default();
    let budget = options.max_bytes.saturating_sub(HEADER_RESERVE) / views.len();
    let mut bodies = Vec::new();
    let mut truncated = false;
    if !hunks.is_empty() {
        for &view in views {
            let (body, cut) = ctx.view(view, budget, &mut flags);
            truncated |= cut;
            bodies.push((view, body));
        }
    }

    let mut main = String::with_capacity(bodies.iter().map(|(_, b)| b.len()).sum::<usize>() + 512);
    if bodies.is_empty() {
        main.push_str(empty_notice(doc.status));
    } else {
        if bodies.len() > 1 {
            main.push_str(concat!(
                "<input type=\"radio\" name=\"view\" id=\"view-split\" class=\"vt\" checked>",
                "<label for=\"view-split\">Side by side</label>\n",
                "<input type=\"radio\" name=\"view\" id=\"view-inline\" class=\"vt\">",
                "<label for=\"view-inline\">Inline</label>\n",
            ));
        }
        for (view, body) in &bodies {
            let (class, label) = match view {
                ViewKind::Split => ("split", "Side-by-side view"),
                ViewKind::Inline => ("inline", "Inline view"),
            };
            let _ = writeln!(
                main,
                "<section class=\"view {class}\" aria-label=\"{label}\">"
            );
            main.push_str(body);
            main.push_str("</section>\n");
        }
    }
    drop(bodies);
    let extra_css = format!(".cn{{width:calc({}ch + 1em)}}\n", ctx.digits);
    Ok(assemble(
        doc,
        options,
        &extra_css,
        &mut flags,
        &main,
        |out, flags| ctx.header(out, flags, truncated, options),
    ))
}

/// The paragraph shown instead of a view when there is nothing to show.
pub(crate) fn empty_notice(status: Status) -> &'static str {
    match status {
        Status::Error => {
            "<p class=\"empty\">No diff to show: the comparison ended with an error.</p>\n"
        }
        _ => "<p class=\"empty\">No changes to show.</p>\n",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewKind {
    Split,
    Inline,
}

impl ViewKind {
    /// Prefix of element ids, so both views can hold their own anchors.
    fn prefix(self) -> &'static str {
        match self {
            ViewKind::Split => "s-",
            ViewKind::Inline => "u-",
        }
    }

    fn columns(self) -> usize {
        match self {
            ViewKind::Split => 6,
            ViewKind::Inline => 4,
        }
    }
}

/// What was seen while rendering, for the notices in the header.
#[derive(Debug, Default)]
pub(crate) struct Flags {
    pub bidi: bool,
    pub invisible: bool,
    pub long_lines: bool,
}

/// How a line is shown in a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellKind {
    Equal,
    Removed,
    Added,
    IgnoredRemoved,
    IgnoredAdded,
}

/// A valid move: `(index in moves[], kind, left range, right range)`, 0-based.
type MoveEntry = (usize, MoveKind, (usize, usize), (usize, usize));

/// Moved blocks by line, per side.
struct MoveIndex {
    /// `(start, count, move index)`, 0-based, sorted by start.
    left: Vec<(usize, usize, usize)>,
    right: Vec<(usize, usize, usize)>,
    /// Valid moves: `(index, kind, left range, right range)`.
    list: Vec<MoveEntry>,
    /// The inline spans of each entry of `list` (`moves[].inline`, spec §6.5).
    spans: Vec<OpSpans>,
}

impl MoveIndex {
    fn new(doc: &DiffDocument, nl: usize, nr: usize) -> MoveIndex {
        let mut index = MoveIndex {
            left: Vec::new(),
            right: Vec::new(),
            list: Vec::new(),
            spans: Vec::new(),
        };
        if doc.status == Status::Error {
            return index;
        }
        for (i, m) in doc.moves.iter().enumerate() {
            let (Some(l), Some(r)) = (range0(m.from.range, nl), range0(m.to.range, nr)) else {
                continue;
            };
            if l.1 == 0 || r.1 == 0 {
                continue;
            }
            index.left.push((l.0, l.1, i));
            index.right.push((r.0, r.1, i));
            index.list.push((i, m.kind, l, r));
            index.spans.push(OpSpans::new(&m.inline));
        }
        index.left.sort_unstable();
        index.right.sort_unstable();
        index
    }

    /// The move holding `line` on `side`, as `(move index, first line of the block)`.
    fn find(&self, side: Side, line: usize) -> Option<(usize, usize)> {
        let list = if side == Side::Left {
            &self.left
        } else {
            &self.right
        };
        let at = list.partition_point(|&(s, n, _)| s + n <= line);
        list.get(at)
            .filter(|&&(s, _, _)| s <= line)
            .map(|&(s, _, i)| (i, s))
    }

    fn get(&self, index: usize) -> Option<&MoveEntry> {
        self.list
            .binary_search_by_key(&index, |e| e.0)
            .ok()
            .map(|at| &self.list[at])
    }

    /// The inline spans of move `index`.
    fn spans(&self, index: usize) -> Option<&OpSpans> {
        self.list
            .binary_search_by_key(&index, |e| e.0)
            .ok()
            .map(|at| &self.spans[at])
    }
}

/// Inline spans of one op, sorted by side, line and start.
struct OpSpans(Vec<(bool, u64, u64, u64)>);

impl OpSpans {
    fn new(spans: &[Span]) -> OpSpans {
        let mut v: Vec<(bool, u64, u64, u64)> = spans
            .iter()
            .filter(|s| matches!(s.side, Side::Left | Side::Right))
            .map(|s| (s.side == Side::Right, s.line, s.start, s.end))
            .collect();
        v.sort_unstable();
        OpSpans(v)
    }

    /// `(start, end)` offsets on 1-based `line` of `side`.
    fn on(&self, right: bool, line: u64) -> impl Iterator<Item = (u64, u64)> + '_ {
        let from = self.0.partition_point(|e| (e.0, e.1) < (right, line));
        self.0[from..]
            .iter()
            .take_while(move |e| (e.0, e.1) == (right, line))
            .map(|e| (e.2, e.3))
    }
}

struct Ctx<'a, 'd> {
    doc: &'d DiffDocument,
    src: &'a Source<'a>,
    hunks: &'a [RHunk<'d>],
    moves: MoveIndex,
    show_eol: bool,
    max_line: usize,
    encoding: PositionEncoding,
    digits: usize,
    /// The view the header's links point into.
    first_view: ViewKind,
    /// Prefix of every element id, so several documents fit on one page.
    ids: &'a str,
    /// A table inside a tree report: fixed line-number columns.
    embedded: bool,
}

impl Ctx<'_, '_> {
    /// Prefix of the element ids of `view`.
    fn anchor(&self, view: ViewKind) -> String {
        format!("{}{}", self.ids, view.prefix())
    }

    fn side(&self, side: Side) -> &SideText<'_> {
        let s = if side == Side::Left {
            &self.src.left
        } else {
            &self.src.right
        };
        // `load` guarantees both sides decode when there are hunks.
        s.as_ref().expect("decoded side")
    }

    /// One view's table. Returns the HTML and whether it was truncated.
    fn view(&self, view: ViewKind, budget: usize, flags: &mut Flags) -> (String, bool) {
        let mut out = String::new();
        let cols = view.columns();
        out.push_str(if self.embedded {
            "<table class=\"diff emb\">\n<colgroup>"
        } else {
            "<table class=\"diff\">\n<colgroup>"
        });
        match view {
            ViewKind::Split => out.push_str(
                "<col class=\"cn\"><col class=\"cm\"><col><col class=\"cn\"><col class=\"cm\"><col>",
            ),
            ViewKind::Inline => {
                out.push_str("<col class=\"cn\"><col class=\"cn\"><col class=\"cm\"><col>");
            }
        }
        out.push_str("</colgroup>\n<thead><tr>");
        let left = self.doc.left.name.as_deref().unwrap_or("left");
        let right = self.doc.right.name.as_deref().unwrap_or("right");
        match view {
            ViewKind::Split => {
                out.push_str("<th scope=\"col\"><span class=\"sr\">Left line</span></th><th scope=\"col\"><span class=\"sr\">Change</span></th><th scope=\"col\">");
                plain(&mut out, left, flags);
                out.push_str("</th><th scope=\"col\"><span class=\"sr\">Right line</span></th><th scope=\"col\"><span class=\"sr\">Change</span></th><th scope=\"col\">");
                plain(&mut out, right, flags);
            }
            ViewKind::Inline => {
                out.push_str("<th scope=\"col\"><span class=\"sr\">Left line</span></th><th scope=\"col\"><span class=\"sr\">Right line</span></th><th scope=\"col\"><span class=\"sr\">Change</span></th><th scope=\"col\">");
                plain(&mut out, left, flags);
                out.push_str(
                    " <span aria-hidden=\"true\">→</span><span class=\"sr\">compared with</span> ",
                );
                plain(&mut out, right, flags);
            }
        }
        out.push_str("</th></tr></thead>\n");

        let (nl, nr) = self.src.counts();
        let (mut end_l, mut end_r) = (0usize, 0usize);
        let mut shown = 0usize;
        let mut cut = false;
        for (i, hunk) in self.hunks.iter().enumerate() {
            if out.len() > budget {
                cut = true;
                break;
            }
            let ((l, ln), (r, rn)) = (hunk.left(), hunk.right());
            self.gap(&mut out, cols, l - end_l, r - end_r);
            let _ = writeln!(
                out,
                "<tbody id=\"{}h{}\"><tr class=\"hh\"><th colspan=\"{cols}\" scope=\"rowgroup\">@@ -{} +{} @@<span class=\"sr\"> hunk {}</span></th></tr>",
                self.anchor(view),
                i + 1,
                header_range((l, ln)),
                header_range((r, rn)),
                i + 1
            );
            let complete = self.hunk_rows(&mut out, view, hunk, budget, flags);
            out.push_str("</tbody>\n");
            if !complete {
                cut = true;
                break;
            }
            shown += 1;
            end_l = l + ln;
            end_r = r + rn;
        }
        if cut {
            let _ = writeln!(
                out,
                "<tbody class=\"cut\"><tr><td colspan=\"{cols}\" role=\"note\">Truncated: the report reached its size limit. {shown} of {} hunks are shown in full; use the unified diff or the JSON document for the rest.</td></tr></tbody>",
                self.hunks.len()
            );
        } else {
            self.gap(&mut out, cols, nl - end_l, nr - end_r);
        }
        out.push_str("</table>\n");
        (out, cut)
    }

    fn gap(&self, out: &mut String, cols: usize, ln: usize, rn: usize) {
        if ln == 0 && rn == 0 {
            return;
        }
        let text = if ln == rn && !self.doc.options.ignore.blank_lines {
            format!(
                "{} not shown",
                text::count(ln as u64, "unchanged line", "unchanged lines")
            )
        } else {
            format!("Left {ln}, right {rn} lines without changes not shown")
        };
        let _ = writeln!(
            out,
            "<tbody class=\"gap\"><tr><td colspan=\"{cols}\">⋯ {text}</td></tr></tbody>"
        );
    }

    /// Rows of one hunk. Returns false when the budget ran out inside it.
    fn hunk_rows(
        &self,
        out: &mut String,
        view: ViewKind,
        hunk: &RHunk<'_>,
        budget: usize,
        flags: &mut Flags,
    ) -> bool {
        for op in &hunk.ops {
            let spans = OpSpans::new(op.inline);
            let (lk, rk) = match op.kind {
                OpKind::Equal => (CellKind::Equal, CellKind::Equal),
                OpKind::Ignored => (CellKind::IgnoredRemoved, CellKind::IgnoredAdded),
                _ => (CellKind::Removed, CellKind::Added),
            };
            match view {
                ViewKind::Split => {
                    for k in 0..op.ln.max(op.rn) {
                        if out.len() > budget {
                            return false;
                        }
                        out.push_str("<tr>");
                        for (side, start, count, kind) in [
                            (Side::Left, op.l, op.ln, lk),
                            (Side::Right, op.r, op.rn, rk),
                        ] {
                            if k < count {
                                let _ = write!(out, "<td class=\"n\">{}</td>", start + k + 1);
                                self.cell(out, view, side, start + k, kind, &spans, flags);
                            } else {
                                out.push_str("<td class=\"n\"></td><td class=\"m x\"></td><td class=\"c x\"></td>");
                            }
                        }
                        out.push_str("</tr>\n");
                    }
                }
                ViewKind::Inline => {
                    if !self.inline_rows(out, op, (lk, rk), &spans, budget, flags) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn inline_rows(
        &self,
        out: &mut String,
        op: &ROp<'_>,
        (lk, rk): (CellKind, CellKind),
        spans: &OpSpans,
        budget: usize,
        flags: &mut Flags,
    ) -> bool {
        if op.kind == OpKind::Equal {
            for k in 0..op.ln {
                if out.len() > budget {
                    return false;
                }
                let _ = write!(
                    out,
                    "<tr><td class=\"n\">{}</td><td class=\"n\">{}</td>",
                    op.l + k + 1,
                    op.r + k + 1
                );
                self.cell(
                    out,
                    ViewKind::Inline,
                    Side::Left,
                    op.l + k,
                    CellKind::Equal,
                    spans,
                    flags,
                );
                out.push_str("</tr>\n");
            }
            return true;
        }
        for k in 0..op.ln {
            if out.len() > budget {
                return false;
            }
            let _ = write!(
                out,
                "<tr><td class=\"n\">{}</td><td class=\"n\"></td>",
                op.l + k + 1
            );
            self.cell(
                out,
                ViewKind::Inline,
                Side::Left,
                op.l + k,
                lk,
                spans,
                flags,
            );
            out.push_str("</tr>\n");
        }
        for k in 0..op.rn {
            if out.len() > budget {
                return false;
            }
            let _ = write!(
                out,
                "<tr><td class=\"n\"></td><td class=\"n\">{}</td>",
                op.r + k + 1
            );
            self.cell(
                out,
                ViewKind::Inline,
                Side::Right,
                op.r + k,
                rk,
                spans,
                flags,
            );
            out.push_str("</tr>\n");
        }
        true
    }

    /// The marker cell and the content cell of one line.
    #[allow(clippy::too_many_arguments)]
    fn cell(
        &self,
        out: &mut String,
        view: ViewKind,
        side: Side,
        line: usize,
        kind: CellKind,
        spans: &OpSpans,
        flags: &mut Flags,
    ) {
        let moved = match kind {
            CellKind::Removed | CellKind::Added => self.moves.find(side, line),
            _ => None,
        };
        let (class, marker) = match (kind, moved.is_some()) {
            (CellKind::Equal, _) => ("", ""),
            (CellKind::Removed, false) => (" d", "−<span class=\"sr\"> removed</span>"),
            (CellKind::Added, false) => (" a", "+<span class=\"sr\"> added</span>"),
            (CellKind::Removed, true) => (" d mv", "⇄<span class=\"sr\"> moved away</span>"),
            (CellKind::Added, true) => (" a mv", "⇄<span class=\"sr\"> moved here</span>"),
            (CellKind::IgnoredRemoved, _) => (" i", "≈<span class=\"sr\"> ignored removal</span>"),
            (CellKind::IgnoredAdded, _) => (" i", "≈<span class=\"sr\"> ignored addition</span>"),
        };
        let _ = write!(
            out,
            "<td class=\"m{class}\">{marker}</td><td class=\"c{class}\">"
        );
        if let Some((index, first)) = moved
            && first == line
        {
            self.move_badge(out, view, side, index);
        }
        let text = self.side(side);
        let content = text.content(line);
        let tag = match kind {
            CellKind::Removed => "del",
            CellKind::Added => "ins",
            _ => "",
        };
        let ranges = if tag.is_empty() {
            Vec::new()
        } else {
            // A moved line shows the spans of its move, every other changed
            // line those of its op (spec §6.5). Spans off the line are
            // ignored either way.
            let spans = moved
                .and_then(|(index, _)| self.moves.spans(index))
                .unwrap_or(spans);
            byte_ranges(
                content,
                spans.on(side == Side::Right, line as u64 + 1),
                self.encoding,
            )
        };
        let changed = kind != CellKind::Equal;
        content_html(out, content, &ranges, tag, changed, self.max_line, flags);
        let term = text.term(line);
        if self.show_eol && changed {
            out.push_str(match term {
                Term::Lf => "<span class=\"eol\" title=\"LF\">␊</span>",
                Term::CrLf => "<span class=\"eol\" title=\"CR LF\">␍␊</span>",
                Term::Cr => "<span class=\"eol\" title=\"CR\">␍</span>",
                Term::None => "",
            });
        }
        if term == Term::None {
            out.push_str("<span class=\"eol\">no newline at end of file</span>");
        }
        out.push_str("</td>");
    }

    fn move_badge(&self, out: &mut String, view: ViewKind, side: Side, index: usize) {
        let Some(&(_, kind, l, r)) = self.moves.get(index) else {
            return;
        };
        let p = self.anchor(view);
        let n = index + 1;
        let edited = if kind == MoveKind::Edited {
            ", edited"
        } else {
            ""
        };
        if side == Side::Left {
            let _ = write!(
                out,
                "<a class=\"mvl\" id=\"{p}m{n}-from\" href=\"#{p}m{n}-to\">moved to right {}{edited}</a>",
                line_range(r)
            );
        } else {
            let _ = write!(
                out,
                "<a class=\"mvl\" id=\"{p}m{n}-to\" href=\"#{p}m{n}-from\">moved from left {}{edited}</a>",
                line_range(l)
            );
        }
    }

    fn header(&self, out: &mut String, flags: &mut Flags, truncated: bool, options: &HtmlOptions) {
        let doc = self.doc;
        files_line(out, doc, flags);
        match doc.status {
            Status::Identical => {
                out.push_str("<p class=\"status st-identical\"><strong>Identical.</strong> No changes under the selected options.");
                if doc.left.sha256 != doc.right.sha256 {
                    out.push_str(" The files are not byte-identical: they differ only in ways the options ignore, such as whitespace, line endings, a byte order mark or masked text.");
                }
                out.push_str("</p>\n");
            }
            Status::Error => out.push_str(ERROR_STATUS),
            _ => {
                out.push_str("<p class=\"status st-different\"><strong>Different.</strong>");
                if let Some(s) = &doc.stats {
                    let _ = write!(
                        out,
                        " {} in {}: {} added, {} removed, {} unchanged; {} similar.",
                        text::count(s.change_count, "change", "changes"),
                        text::count(s.hunk_count, "hunk", "hunks"),
                        text::count(s.lines_added, "line", "lines"),
                        s.lines_removed,
                        s.lines_unchanged,
                        text::percent(s.similarity.basis_points())
                    );
                }
                out.push_str("</p>\n");
            }
        }
        if truncated {
            out.push_str("<p class=\"notice\" role=\"note\"><strong>Truncated.</strong> This report reached its size limit; not every hunk is shown.</p>\n");
        }
        char_notices(out, flags);
        if flags.long_lines {
            let _ = writeln!(
                out,
                "<p class=\"notice\" role=\"note\"><strong>Long lines.</strong> Lines longer than {} bytes are cut short.</p>",
                options.max_line_bytes
            );
        }
        out.push_str(concat!(
            "<ul class=\"legend\" aria-label=\"Legend\">",
            "<li><span class=\"key d\">−</span> removed</li>",
            "<li><span class=\"key a\">+</span> added</li>",
            "<li><span class=\"key mv\">⇄</span> moved</li>",
            "<li><span class=\"key i\">≈</span> ignored</li>",
            "<li><del>word</del> <ins>word</ins> changed text</li>",
            "<li><span class=\"inv\">RLO</span> invisible or control character</li>",
            "</ul>\n"
        ));
        self.details(out, flags, options);
        warnings_section(out, &doc.warnings);
        self.moves_list(out);
    }

    fn details(&self, out: &mut String, flags: &mut Flags, options: &HtmlOptions) {
        let (l, r) = (&self.doc.left, &self.doc.right);
        out.push_str("<details>\n<summary>Files and options</summary>\n<table class=\"meta\">\n<thead><tr><td></td><th scope=\"col\">Left</th><th scope=\"col\">Right</th></tr></thead>\n<tbody>\n");
        out.push_str("<tr><th scope=\"row\">Name</th><td>");
        plain(out, l.name.as_deref().unwrap_or("(none)"), flags);
        out.push_str("</td><td>");
        plain(out, r.name.as_deref().unwrap_or("(none)"), flags);
        out.push_str("</td></tr>\n");
        let rows: [MetaRow; 7] = [
            ("Size", |m| text::count(m.size, "byte", "bytes")),
            ("Lines", |m| {
                m.line_count
                    .map_or_else(|| "—".to_owned(), |n| n.to_string())
            }),
            ("Encoding", |m| {
                m.encoding
                    .clone()
                    .unwrap_or_else(|| "not valid UTF-8".to_owned())
            }),
            ("Line endings", |m| {
                terminator_name(m.line_terminator).to_owned()
            }),
            ("Final newline", |m| yes_no(m.final_newline).to_owned()),
            ("Byte order mark", |m| yes_no(Some(m.bom)).to_owned()),
            ("SHA-256", |m| m.sha256.clone()),
        ];
        for (name, f) in rows {
            let _ = write!(out, "<tr><th scope=\"row\">{name}</th><td>");
            let code = name == "SHA-256";
            if code {
                out.push_str("<code>");
            }
            push_escaped(out, &f(l));
            if code {
                out.push_str("</code>");
            }
            out.push_str("</td><td>");
            if code {
                out.push_str("<code>");
            }
            push_escaped(out, &f(r));
            if code {
                out.push_str("</code>");
            }
            out.push_str("</td></tr>\n");
        }
        out.push_str("</tbody>\n</table>\n<p>Options: ");
        push_escaped(out, &self.options_text(options));
        out.push_str(".</p>\n</details>\n");
    }

    fn options_text(&self, options: &HtmlOptions) -> String {
        let o = &self.doc.options;
        let algorithm = match o.algorithm {
            Algorithm::Histogram => "histogram",
            Algorithm::Myers => "myers",
            Algorithm::Patience => "patience",
            Algorithm::Unknown => "unknown",
        };
        let granularity = match o.granularity {
            Granularity::Line => "line",
            Granularity::Word => "word",
            Granularity::Char => "char",
            Granularity::Smart => "smart",
            Granularity::Unknown => "unknown",
        };
        let whitespace = match o.ignore.whitespace {
            WhitespaceMode::None => "compared",
            WhitespaceMode::Trailing => "trailing ignored",
            WhitespaceMode::Change => "changes in amount ignored",
            WhitespaceMode::All => "all ignored",
            WhitespaceMode::Unknown => "unknown",
        };
        let positions = match o.position_encoding {
            PositionEncoding::Utf8 => "utf-8",
            PositionEncoding::Utf16 => "utf-16",
            PositionEncoding::Codepoint => "codepoint",
            PositionEncoding::Unknown => "unknown",
        };
        let mut parts = vec![
            format!("algorithm {algorithm}"),
            format!("granularity {granularity}"),
            format!("whitespace {whitespace}"),
            format!(
                "case {}",
                if o.ignore.case { "ignored" } else { "compared" }
            ),
            format!(
                "blank lines {}",
                if o.ignore.blank_lines {
                    "ignored"
                } else {
                    "compared"
                }
            ),
            format!(
                "{} masked",
                text::count(o.ignore.regex.len() as u64, "pattern", "patterns")
            ),
            format!(
                "moves {}",
                if o.detect_moves {
                    "detected"
                } else {
                    "not detected"
                }
            ),
            format!("positions {positions}"),
        ];
        match options.context {
            Some(c) if c != o.context => {
                parts.push(format!("context {} (shown with {c})", o.context));
            }
            _ => parts.push(format!("context {}", o.context)),
        }
        parts.join(", ")
    }

    fn moves_list(&self, out: &mut String) {
        if self.moves.list.is_empty() {
            return;
        }
        let p = self.anchor(self.first_view);
        out.push_str("<section aria-labelledby=\"moves-h\">\n<h2 id=\"moves-h\">Moved blocks</h2>\n<ol class=\"moves\">\n");
        for &(index, kind, l, r) in self.moves.list.iter().take(MAX_LISTED_MOVES) {
            let n = index + 1;
            let kind = match kind {
                MoveKind::Pure => "unchanged",
                MoveKind::Edited => "edited",
                MoveKind::Unknown => "unknown kind",
            };
            let _ = writeln!(
                out,
                "<li>Left lines <a href=\"#{p}m{n}-from\">{}</a> moved to right lines <a href=\"#{p}m{n}-to\">{}</a> ({kind})</li>",
                line_range(l),
                line_range(r)
            );
        }
        out.push_str("</ol>\n");
        if self.moves.list.len() > MAX_LISTED_MOVES {
            let _ = writeln!(
                out,
                "<p>{} more moved blocks are not listed.</p>",
                self.moves.list.len() - MAX_LISTED_MOVES
            );
        }
        out.push_str("</section>\n");
    }
}

/// `3–6`, or `3` for one line (1-based, from a 0-based `(start, count)`).
fn line_range((start, count): (usize, usize)) -> String {
    if count <= 1 {
        format!("{}", start + 1)
    } else {
        format!("{}–{}", start + 1, start + count)
    }
}

fn terminator_name(t: Option<LineTerminator>) -> &'static str {
    match t {
        Some(LineTerminator::Lf) => "LF",
        Some(LineTerminator::Crlf) => "CRLF",
        Some(LineTerminator::Cr) => "CR",
        Some(LineTerminator::Mixed) => "mixed",
        Some(LineTerminator::None) => "none",
        Some(LineTerminator::Unknown) | None => "—",
    }
}

fn yes_no(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "yes",
        Some(false) => "no",
        None => "—",
    }
}

/// Converts span offsets (in `encoding` units) to sorted, non-overlapping
/// byte ranges of `content`. Offsets inside a character move to its end;
/// offsets past the end are clamped, and empty ranges are dropped.
fn byte_ranges(
    content: &str,
    spans: impl Iterator<Item = (u64, u64)>,
    encoding: PositionEncoding,
) -> Vec<(usize, usize)> {
    let spans: Vec<(u64, u64)> = spans.collect();
    if spans.is_empty() {
        return Vec::new();
    }
    let mut bounds: Vec<u64> = spans.iter().flat_map(|&(s, e)| [s, e]).collect();
    bounds.sort_unstable();
    bounds.dedup();
    let mut map: Vec<(u64, usize)> = Vec::with_capacity(bounds.len());
    let mut next = 0;
    let mut unit = 0u64;
    for (byte, c) in content.char_indices() {
        while next < bounds.len() && bounds[next] <= unit {
            map.push((bounds[next], byte));
            next += 1;
        }
        if next == bounds.len() {
            break;
        }
        unit += match encoding {
            PositionEncoding::Utf16 => c.len_utf16() as u64,
            PositionEncoding::Codepoint => 1,
            _ => c.len_utf8() as u64,
        };
    }
    for &b in &bounds[next..] {
        map.push((b, content.len()));
    }
    let byte_of = |u: u64| {
        map.binary_search_by_key(&u, |e| e.0)
            .map_or(content.len(), |at| map[at].1)
    };
    let mut ranges: Vec<(usize, usize)> = spans
        .iter()
        .map(|&(s, e)| (byte_of(s), byte_of(e)))
        .filter(|(s, e)| s < e)
        .collect();
    ranges.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (s, e) in ranges {
        match out.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// Writes line content: escaped, with `tag` (`del` or `ins`) around the byte
/// `ranges`, invisible characters as labels, and (for changed lines) tabs and
/// trailing whitespace marked. Content past `max` bytes is cut.
fn content_html(
    out: &mut String,
    content: &str,
    ranges: &[(usize, usize)],
    tag: &str,
    changed: bool,
    max: usize,
    flags: &mut Flags,
) {
    let trail = if changed {
        content.trim_end_matches([' ', '\t']).len()
    } else {
        content.len()
    };
    let mut state = (false, false);
    let mut next = 0;
    let mut cut = None;
    for (i, c) in content.char_indices() {
        if i >= max {
            cut = Some(content.len() - i);
            break;
        }
        while next < ranges.len() && ranges[next].1 <= i {
            next += 1;
        }
        let in_span = !tag.is_empty() && next < ranges.len() && ranges[next].0 <= i;
        let now = (in_span, i >= trail);
        if now != state {
            close(out, state, tag);
            if now.0 {
                let _ = write!(out, "<{tag}>");
            }
            if now.1 {
                out.push_str("<span class=\"tw\" title=\"trailing whitespace\">");
            }
            state = now;
        }
        if c == '\t' && changed {
            out.push_str("<span class=\"tab\">\t</span>");
        } else {
            push_char(out, c, flags);
        }
    }
    close(out, state, tag);
    if let Some(rest) = cut {
        flags.long_lines = true;
        let _ = write!(
            out,
            "<span class=\"cl\">… {} not shown</span>",
            text::count(rest as u64, "more byte", "more bytes")
        );
    }
}

fn close(out: &mut String, (span, trail): (bool, bool), tag: &str) {
    if trail {
        out.push_str("</span>");
    }
    if span {
        let _ = write!(out, "</{tag}>");
    }
}

/// Writes text escaped, with invisible characters as labels.
pub(crate) fn plain(out: &mut String, s: &str, flags: &mut Flags) {
    for c in s.chars() {
        push_char(out, c, flags);
    }
}

fn push_char(out: &mut String, c: char, flags: &mut Flags) {
    if let Some(inv) = text::invisible(c) {
        flags.invisible = true;
        flags.bidi |= inv.bidi;
        let _ = write!(
            out,
            "<span class=\"inv\" title=\"{}\">{}</span>",
            text::code_point(c),
            text::invisible_label(c, inv)
        );
        return;
    }
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&#39;"),
        c => out.push(c),
    }
}

const CSS: &str = r#":root{color-scheme:light dark;--bg:#fff;--fg:#1f2328;--mute:#59636e;--bd:#d1d9e0;--hh:#eef3f8;--gap:#f6f8fa;--a-bg:#e1efff;--a-hi:#a8cdf5;--a-fg:#0b4f8a;--d-bg:#ffead9;--d-hi:#f4bd8f;--d-fg:#8f3f00;--mv:#7b3aa6;--mv-bg:#f4eafb;--i-bg:#f0f1f3;--inv-bg:#fff1a8;--inv-fg:#5a4500;--link:#0b5cad}
@media (prefers-color-scheme:dark){:root{--bg:#0d1117;--fg:#e6edf3;--mute:#9198a1;--bd:#3d444d;--hh:#161d27;--gap:#151b23;--a-bg:#0c2d4d;--a-hi:#1d5d99;--a-fg:#a8cdf5;--d-bg:#3a230f;--d-hi:#7c4517;--d-fg:#f4bd8f;--mv:#d2a8ef;--mv-bg:#2a1c36;--i-bg:#1b1f24;--inv-bg:#5a4500;--inv-fg:#fff1a8;--link:#79b8ff}}
*{box-sizing:border-box}
body{margin:0;padding:1rem 1.25rem 2rem;background:var(--bg);color:var(--fg);font:15px/1.5 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
a{color:var(--link)}
h1{font-size:1.35rem;margin:0 0 .25rem;overflow-wrap:anywhere}
h2{font-size:1.05rem;margin:1rem 0 .4rem}
code,.diff{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,"Liberation Mono",monospace}
.sr{position:absolute;width:1px;height:1px;margin:-1px;padding:0;overflow:hidden;clip:rect(0 0 0 0);white-space:nowrap;border:0}
.files{margin:0;color:var(--mute);overflow-wrap:anywhere}
.notice{border:1px solid var(--bd);border-left:4px solid var(--mv);padding:.4rem .75rem;background:var(--gap)}
.legend{display:flex;flex-wrap:wrap;gap:.25rem 1.25rem;list-style:none;padding:0;margin:.5rem 0;font-size:.9rem}
.key{display:inline-block;min-width:1.7em;text-align:center;font-weight:700;border:1px solid var(--bd);border-radius:3px}
.key.d{background:var(--d-bg);color:var(--d-fg)}.key.a{background:var(--a-bg);color:var(--a-fg)}.key.mv{background:var(--mv-bg);color:var(--mv)}.key.i{background:var(--i-bg);color:var(--mute)}
details{margin:.5rem 0}summary{cursor:pointer}
table.meta{border-collapse:collapse;font-size:.85rem;margin:.5rem 0}
.meta th,.meta td{border:1px solid var(--bd);padding:.15rem .5rem;text-align:left;vertical-align:top;overflow-wrap:anywhere}
.warnings{padding-left:1.25rem}.sev-error strong{color:var(--d-fg)}
.empty{font-size:1.05rem;padding:1rem;border:1px solid var(--bd);background:var(--gap)}
.vt{position:absolute;opacity:0;width:1px;height:1px}
.vt+label{display:inline-block;padding:.2rem .8rem;border:1px solid var(--bd);border-radius:4px;margin:0 .3rem .6rem 0;cursor:pointer}
.vt:checked+label{background:var(--hh);border-color:var(--fg);font-weight:600}
.vt:focus-visible+label{outline:2px solid var(--link);outline-offset:2px}
#view-split:checked~.inline,#view-inline:checked~.split{display:none}
table.diff{width:100%;border-collapse:collapse;table-layout:fixed;font-size:13px;line-height:1.45;border:1px solid var(--bd)}
.cm{width:1.6em}
.diff thead th{background:var(--hh);text-align:left;font-weight:600;padding:.25rem .5rem;border-bottom:1px solid var(--bd);overflow-wrap:anywhere}
.diff td{padding:0 .4rem;vertical-align:top}
.diff .n{color:var(--mute);text-align:right;-webkit-user-select:none;user-select:none}
.diff .m{padding:0;text-align:center;font-weight:700;-webkit-user-select:none;user-select:none}
.diff .c{white-space:pre-wrap;overflow-wrap:anywhere;unicode-bidi:plaintext;tab-size:4}
.diff .hh th{background:var(--hh);color:var(--mute);text-align:left;font-weight:400;padding:.2rem .5rem;border-top:1px solid var(--bd);border-bottom:1px solid var(--bd)}
.diff .gap td{background:var(--gap);color:var(--mute);text-align:center;font-style:italic;padding:.1rem}
.diff .cut td{background:var(--inv-bg);color:var(--inv-fg);text-align:center;padding:.4rem}
.d{background:var(--d-bg)}.m.d{color:var(--d-fg)}
.a{background:var(--a-bg)}.m.a{color:var(--a-fg)}
.i{background:var(--i-bg);color:var(--mute)}
.x{background:var(--gap)}
.mv{background:var(--mv-bg)}.m.mv{color:var(--mv)}.c.mv{box-shadow:inset 3px 0 0 var(--mv)}
del{background:var(--d-hi);color:inherit;text-decoration:line-through 1px}
ins{background:var(--a-hi);color:inherit;text-decoration:underline 2px;text-underline-offset:2px}
.inv{display:inline-block;font:600 .7em/1.2 system-ui,sans-serif;padding:0 .2em;margin:0 .05em;border:1px solid var(--inv-fg);border-radius:3px;background:var(--inv-bg);color:var(--inv-fg);vertical-align:.1em;text-decoration:none;unicode-bidi:isolate}
.tab{border-bottom:1px dotted var(--mute)}
.tw{outline:1px dashed var(--mute);outline-offset:-1px}
.eol{margin-left:.35em;font:.75em system-ui,sans-serif;color:var(--mute);-webkit-user-select:none;user-select:none}
.cl{font:italic .8em system-ui,sans-serif;color:var(--mute)}
.mvl{float:right;margin-left:.5em;padding:0 .35em;font:.75em/1.6 system-ui,sans-serif;border:1px solid var(--mv);border-radius:3px;color:var(--mv);text-decoration:none;-webkit-user-select:none;user-select:none}
.mvl:focus-visible,.mvl:hover{text-decoration:underline}
footer{margin-top:1.5rem;color:var(--mute);font-size:.85rem;overflow-wrap:anywhere}
@media print{.vt,.vt+label{display:none}.view{display:block!important}}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(v: &[(u64, u64)]) -> impl Iterator<Item = (u64, u64)> + '_ {
        v.iter().copied()
    }

    #[test]
    fn byte_ranges_per_encoding() {
        // "aé😀b": a=1 byte, é=2 bytes/1 unit, 😀=4 bytes/2 UTF-16 units/1 code point.
        let s = "aé😀b";
        assert_eq!(
            byte_ranges(s, spans(&[(1, 3)]), PositionEncoding::Utf8),
            [(1, 3)]
        );
        assert_eq!(
            byte_ranges(s, spans(&[(2, 4)]), PositionEncoding::Utf16),
            [(3, 7)]
        );
        assert_eq!(
            byte_ranges(s, spans(&[(2, 3)]), PositionEncoding::Codepoint),
            [(3, 7)]
        );
        // Inside a character: moved to its end. Past the end: clamped.
        assert_eq!(
            byte_ranges(s, spans(&[(2, 99)]), PositionEncoding::Utf8),
            [(3, 8)]
        );
        assert!(byte_ranges(s, spans(&[(3, 4)]), PositionEncoding::Utf16).is_empty());
        // Overlaps merge, empty ranges drop.
        assert_eq!(
            byte_ranges(s, spans(&[(0, 2), (1, 3), (5, 5)]), PositionEncoding::Utf8),
            [(0, 3)]
        );
    }

    #[test]
    fn content_escapes_and_marks() {
        let mut out = String::new();
        let mut flags = Flags::default();
        content_html(
            &mut out,
            "<b>x\u{202e}y\t ",
            &[(1, 2)],
            "ins",
            true,
            1000,
            &mut flags,
        );
        assert_eq!(
            out,
            "&lt;<ins>b</ins>&gt;x<span class=\"inv\" title=\"U+202E\">RLO</span>y<span class=\"tw\" title=\"trailing whitespace\"><span class=\"tab\">\t</span> </span>"
        );
        assert!(flags.bidi && flags.invisible && !flags.long_lines);
    }

    #[test]
    fn long_content_is_cut() {
        let mut out = String::new();
        let mut flags = Flags::default();
        content_html(&mut out, "ééé", &[], "", false, 3, &mut flags);
        assert_eq!(out, "éé<span class=\"cl\">… 2 more bytes not shown</span>");
        assert!(flags.long_lines);
    }
}
