//! HTML for kind `tree` (spec §6.8, §19): the entries, the warnings that
//! matter for folders and archives, and each modified entry's child
//! document inline.

use std::fmt::Write as _;

use diffyman_model::{
    ChildDocument, DiffDocument, EntrySide, EntryStatus, EntryType, Kind, OpKind, Range, Side,
    Status, TreeEntry, WarningCode,
};

use crate::RenderError;
use crate::hex::{self, HexBudget};
use crate::html::{self, EntryContents, Flags, HtmlOptions, plain};
use crate::source;
use crate::summary;
use crate::text::{self, push_escaped};

/// Styles of the entries table, on top of [`hex::CSS`].
const CSS: &str = r".entries .p{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
.key.ch{background:var(--hh);color:var(--fg)}.key.tc{background:var(--inv-bg);color:var(--inv-fg)}.key.eq{background:var(--gap);color:var(--mute)}
.badge{display:inline-block;margin-left:.4em;padding:0 .35em;font:.75em/1.6 system-ui,sans-serif;border:1px solid var(--bd);border-radius:3px;color:var(--mute);vertical-align:.1em}
.badge.bad{border-color:var(--d-fg);color:var(--d-fg);font-weight:700}
.notice.flag{border-left-color:var(--d-fg)}
.mute{color:var(--mute)}
.entries tr.child>td{border-top:0;padding-top:0}
.child details{margin:0 0 .4rem}
.child summary{color:var(--mute)}
.child .warnings{margin:.25rem 0;font-size:.85rem}
.note{color:var(--mute);font-style:italic;margin:.25rem 0}
.hunks{margin:.25rem 0;padding-left:1.25rem}
.diff.emb{margin:.25rem 0}.diff.emb .cn{width:calc(8ch + 1em)}
details.same{margin-top:.75rem}
.st-partial strong{color:var(--d-fg)}
";

const TABLE_HEAD: &str = "<table class=\"entries\">\n<thead><tr><th scope=\"col\">Status</th><th scope=\"col\">Path</th><th scope=\"col\">Left</th><th scope=\"col\">Right</th><th scope=\"col\">SHA-256</th></tr></thead>\n<tbody>\n";

const LEGEND: &str = concat!(
    "<ul class=\"legend\" aria-label=\"Legend\">",
    "<li><span class=\"key a\">+</span> added</li>",
    "<li><span class=\"key d\">−</span> removed</li>",
    "<li><span class=\"key ch\">~</span> modified</li>",
    "<li><span class=\"key mv\">→</span> moved (same content, new path)</li>",
    "<li><span class=\"key tc\">≠</span> type changed</li>",
    "<li><span class=\"key eq\">=</span> unchanged</li>",
    "<li><span class=\"badge bad\">unsafe path</span> absolute or climbing out of the tree</li>",
    "<li><span class=\"inv\">RLO</span> invisible or control character</li>",
    "</ul>\n"
);

/// The tree warnings flagged at the top of the report: title and
/// explanation (HTML).
const FLAGGED: &[(WarningCode, &str, &str)] = &[
    (
        WarningCode::UnsafePath,
        "Unsafe paths",
        "Some entry paths are absolute, start with a drive letter or climb out of the tree with <code>..</code>. They are marked <span class=\"badge bad\">unsafe path</span> and listed as given: they were never resolved, and nothing was read from or written to them.",
    ),
    (
        WarningCode::EncryptedEntry,
        "Encrypted entries",
        "Some archive entries are encrypted. They were not decrypted and are compared by their stored bytes only.",
    ),
    (
        WarningCode::EntryUnreadable,
        "Unreadable entries",
        "Some entries could not be read; they are compared without their content.",
    ),
    (
        WarningCode::TreeLimit,
        "Limits reached",
        "Some entries were not read because they exceed folder or archive limits (size, compression ratio, overlapping data or path length).",
    ),
    (
        WarningCode::DuplicatePath,
        "Duplicate paths",
        "Some paths appear more than once on one side; one entry per path was kept.",
    ),
    (
        WarningCode::NonUtf8Path,
        "Names that are not UTF-8",
        "Invalid bytes in some names were replaced with U+FFFD.",
    ),
    (
        WarningCode::ChildrenOmitted,
        "Child documents left out",
        "Some modified files have no child document because it exceeds size limits.",
    ),
    (
        WarningCode::DepthLimit,
        "Nesting limit",
        "Archives nested deeper than <code>tree.maxDepth</code> were compared as bytes.",
    ),
];

/// What happened while rendering, for the notices in the header.
#[derive(Debug, Default)]
struct Notes {
    /// Rows were left out by the size cap.
    truncated: bool,
    hex_cut: bool,
    /// Child documents below `max_depth` were not shown.
    depth_cut: bool,
    /// `max_content_bytes` was reached.
    content_cut: bool,
    /// Some child was shown without content.
    no_content: bool,
    /// Some supplied content did not match its child document.
    mismatch: bool,
}

struct Report<'a> {
    options: &'a HtmlOptions,
    contents: &'a mut dyn EntryContents,
    flags: Flags,
    hex: HexBudget,
    /// What is left of `max_content_bytes`.
    content_left: u64,
    /// Number for the element ids of the next embedded text view.
    next_id: usize,
    notes: Notes,
}

/// The page of a tree document. The inputs are not read: everything shown
/// comes from the document, and entry contents from `contents`.
pub(crate) fn page(
    doc: &DiffDocument,
    contents: &mut dyn EntryContents,
    options: &HtmlOptions,
) -> Result<String, RenderError> {
    if doc.status != Status::Error && doc.tree.is_none() {
        return Err(RenderError::InvalidDocument(
            "a tree document has no tree block".to_owned(),
        ));
    }
    let mut report = Report {
        options,
        contents,
        flags: Flags::default(),
        hex: HexBudget::new(options),
        content_left: options.max_content_bytes,
        next_id: 1,
        notes: Notes::default(),
    };
    let budget = options.max_bytes.saturating_sub(html::HEADER_RESERVE);
    let mut main = String::new();
    if doc.status == Status::Error {
        main.push_str(html::empty_notice(doc.status));
    } else {
        main.push_str("<h2>Entries</h2>\n");
        report.entries(&mut main, doc, &[], 0, budget);
    }
    let Report {
        mut flags,
        hex,
        mut notes,
        ..
    } = report;
    notes.hex_cut = hex.cut;
    let css = format!("{}{CSS}", hex::CSS);
    Ok(html::assemble(
        doc,
        options,
        &css,
        &mut flags,
        &main,
        |out, flags| header(out, doc, flags, &notes, options),
    ))
}

fn header(
    out: &mut String,
    doc: &DiffDocument,
    flags: &mut Flags,
    notes: &Notes,
    options: &HtmlOptions,
) {
    html::files_line(out, doc, flags);
    match doc.status {
        Status::Identical => out.push_str("<p class=\"status st-identical\"><strong>Identical.</strong> Every entry is unchanged.</p>\n"),
        Status::Error => out.push_str(html::ERROR_STATUS),
        Status::Partial => {
            out.push_str("<p class=\"status st-partial\"><strong>Partial.</strong> Some content was not read, so equal content cannot be confirmed for every entry. ");
            out.push_str(&counts(doc));
            out.push_str("</p>\n");
        }
        _ => {
            out.push_str("<p class=\"status st-different\"><strong>Different.</strong> ");
            out.push_str(&counts(doc));
            out.push_str("</p>\n");
        }
    }
    for w in &doc.warnings {
        let Some((_, title, explanation)) = FLAGGED.iter().find(|f| f.0 == w.code) else {
            continue;
        };
        let side = match w.side {
            Some(Side::Left) => " (left)",
            Some(Side::Right) => " (right)",
            _ => "",
        };
        let _ = writeln!(
            out,
            "<p class=\"notice flag\" role=\"note\"><span aria-hidden=\"true\">⚠ </span><strong>{title}{side}.</strong> {explanation}</p>"
        );
    }
    if notes.truncated {
        out.push_str("<p class=\"notice\" role=\"note\"><strong>Truncated.</strong> This report reached its size limit; not every entry or change is shown.</p>\n");
    }
    if notes.hex_cut {
        hex::hex_notice(out, options);
    }
    if notes.depth_cut {
        let _ = writeln!(
            out,
            "<p class=\"notice\" role=\"note\"><strong>Nesting.</strong> Child documents more than {} deep are not shown.</p>",
            text::count(u64::from(options.max_depth), "level", "levels")
        );
    }
    if notes.content_cut {
        let _ = writeln!(
            out,
            "<p class=\"notice\" role=\"note\"><strong>Content limit.</strong> This report reads at most {} of entry content; later entries are shown without it.</p>",
            text::count(options.max_content_bytes, "byte", "bytes")
        );
    }
    if notes.no_content {
        out.push_str("<p class=\"notice\" role=\"note\"><strong>Positions only.</strong> The content of some modified entries was not available to this report, so their changes are listed by line number or byte offset.</p>\n");
    }
    if notes.mismatch {
        out.push_str("<p class=\"notice\" role=\"note\"><strong>Content mismatch.</strong> The content supplied for some entries does not match the document, so it is not shown.</p>\n");
    }
    html::char_notices(out, flags);
    out.push_str(LEGEND);
    out.push_str("<details>\n<summary>Sides and options</summary>\n");
    sides_table(out, doc, flags);
    out.push_str("<p>Options: ");
    push_escaped(out, &options_text(doc));
    out.push_str(".</p>\n</details>\n");
    html::warnings_section(out, &doc.warnings);
}

/// `N changed entries: A added, R removed, ...; U unchanged; S similar.`
fn counts(doc: &DiffDocument) -> String {
    let Some(stats) = &doc.stats else {
        return String::new();
    };
    let mut out = text::count(stats.change_count, "changed entry", "changed entries");
    if let Some(t) = &stats.tree {
        let _ = write!(
            out,
            ": {} added, {} removed, {} modified, {} moved, {}; {} unchanged",
            t.added,
            t.removed,
            t.modified,
            t.moved,
            text::count(t.type_changed, "type change", "type changes"),
            t.unchanged
        );
    }
    let _ = write!(
        out,
        "; {} similar.",
        text::percent(stats.similarity.basis_points())
    );
    out
}

fn options_text(doc: &DiffDocument) -> String {
    let t = &doc.options.tree;
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    format!(
        "moves detected {}, symbolic links followed {}, ignore files used {}, {} excluded, max depth {}, child budget {}",
        yes_no(t.detect_moves),
        yes_no(t.follow_symlinks),
        yes_no(t.ignore_files),
        text::count(t.exclude.len() as u64, "pattern", "patterns"),
        t.max_depth,
        text::count(u64::from(t.child_budget), "byte", "bytes")
    )
}

/// Name, type, entries, size and digest of both sides.
fn sides_table(out: &mut String, doc: &DiffDocument, flags: &mut Flags) {
    let tree = doc.stats.as_ref().and_then(|s| s.tree.as_ref());
    out.push_str("<table class=\"meta\">\n<thead><tr><td></td><th scope=\"col\">Left</th><th scope=\"col\">Right</th></tr></thead>\n<tbody>\n");
    let rows: [(&str, [String; 2]); 5] = [
        (
            "Name",
            [&doc.left, &doc.right].map(|m| m.name.clone().unwrap_or_else(|| "(none)".to_owned())),
        ),
        (
            "Type",
            [&doc.left, &doc.right].map(|m| match m.media_type.as_deref() {
                Some("inode/directory") => "folder".to_owned(),
                Some(media) => media.to_owned(),
                None => "unknown".to_owned(),
            }),
        ),
        (
            "Entries",
            [tree.map(|t| t.entries_left), tree.map(|t| t.entries_right)]
                .map(|n| n.map_or_else(|| "—".to_owned(), |n| n.to_string())),
        ),
        (
            "Size",
            [&doc.left, &doc.right].map(|m| text::count(m.size, "byte", "bytes")),
        ),
        ("SHA-256", [&doc.left, &doc.right].map(|m| m.sha256.clone())),
    ];
    for (name, values) in rows {
        let _ = write!(out, "<tr><th scope=\"row\">{name}</th>");
        for value in values {
            out.push_str("<td>");
            if name == "SHA-256" {
                out.push_str("<code>");
                plain(out, &value, flags);
                out.push_str("</code>");
            } else {
                plain(out, &value, flags);
            }
            out.push_str("</td>");
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n");
    if [&doc.left, &doc.right]
        .iter()
        .any(|m| m.media_type.as_deref() == Some("inode/directory"))
    {
        out.push_str("<p>The SHA-256 of a folder is the digest of its list of paths, types and contents (its manifest).</p>\n");
    }
}

/// How a status looks: CSS class, icon and label.
fn status_look(status: EntryStatus) -> (&'static str, &'static str, &'static str) {
    match status {
        EntryStatus::Added => ("a", "+", "added"),
        EntryStatus::Removed => ("d", "−", "removed"),
        EntryStatus::Modified => ("ch", "~", "modified"),
        EntryStatus::Moved => ("mv", "→", "moved"),
        EntryStatus::TypeChange => ("tc", "≠", "type changed"),
        EntryStatus::Unchanged => ("eq", "=", "unchanged"),
        EntryStatus::Unknown => ("eq", "?", "unknown status"),
    }
}

/// A path that is absolute, starts with a drive letter or has a `..`
/// component (spec §19.2 rule 7).
fn unsafe_path(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        || path.split(['/', '\\']).any(|c| c == "..")
}

/// `l,s` of a hunk header from a 1-based range, as GNU writes it.
fn range_text(r: Range) -> String {
    match r.count {
        0 => format!("{},0", r.start.saturating_sub(1)),
        1 => r.start.to_string(),
        n => format!("{},{n}", r.start),
    }
}

/// A file entry's digest, as shown.
enum Digest<'a> {
    None,
    NotRead,
    Sha(&'a str),
}

fn digest(side: Option<&EntrySide>) -> Digest<'_> {
    match side {
        Some(s) if s.entry_type == EntryType::File => {
            s.sha256.as_deref().map_or(Digest::NotRead, Digest::Sha)
        }
        _ => Digest::None,
    }
}

impl Report<'_> {
    /// The entries of `doc` (a tree document): the changed ones with their
    /// child documents, then the unchanged ones in a collapsed list. Rows
    /// stop once this part passes `budget` bytes.
    fn entries(
        &mut self,
        out: &mut String,
        doc: &DiffDocument,
        chain: &[&str],
        depth: u32,
        budget: usize,
    ) {
        let Some(tree) = &doc.tree else {
            return;
        };
        let start = out.len();
        let changed = tree
            .entries
            .iter()
            .filter(|e| e.status != EntryStatus::Unchanged)
            .count();
        let unchanged = tree.entries.len() - changed;
        let mut cut = false;
        if changed == 0 {
            out.push_str("<p class=\"note\">No changed entries.</p>\n");
        } else {
            out.push_str(TABLE_HEAD);
            let mut shown = 0usize;
            for e in tree
                .entries
                .iter()
                .filter(|e| e.status != EntryStatus::Unchanged)
            {
                if out.len() - start > budget {
                    cut = true;
                    break;
                }
                self.row(out, e);
                if e.status == EntryStatus::Modified {
                    let left = budget.saturating_sub(out.len() - start);
                    self.child_row(out, doc, e, chain, depth, left);
                }
                shown += 1;
            }
            if cut {
                let _ = writeln!(
                    out,
                    "<tr class=\"cut\"><td colspan=\"5\" role=\"note\">Truncated: the report reached its size limit. {shown} of {changed} changed entries are listed; use the JSON document for the rest.</td></tr>"
                );
            }
            out.push_str("</tbody>\n</table>\n");
        }
        if unchanged > 0 && !cut {
            let _ = writeln!(
                out,
                "<details class=\"same\">\n<summary>{}</summary>",
                text::count(unchanged as u64, "unchanged entry", "unchanged entries")
            );
            out.push_str(TABLE_HEAD);
            let mut shown = 0usize;
            for e in tree
                .entries
                .iter()
                .filter(|e| e.status == EntryStatus::Unchanged)
            {
                if out.len() - start > budget {
                    cut = true;
                    break;
                }
                self.row(out, e);
                shown += 1;
            }
            if cut {
                let _ = writeln!(
                    out,
                    "<tr class=\"cut\"><td colspan=\"5\" role=\"note\">Truncated: the report reached its size limit. {shown} of {unchanged} unchanged entries are listed.</td></tr>"
                );
            }
            out.push_str("</tbody>\n</table>\n</details>\n");
        }
        self.notes.truncated |= cut;
    }

    fn row(&mut self, out: &mut String, e: &TreeEntry) {
        let (class, icon, label) = status_look(e.status);
        let _ = write!(
            out,
            "<tr><td class=\"st\"><span class=\"key {class}\" aria-hidden=\"true\">{icon}</span> {label}</td><td class=\"p\">"
        );
        let from = e
            .from_path
            .as_deref()
            .filter(|_| e.status == EntryStatus::Moved);
        if let Some(from) = from {
            plain(out, from, &mut self.flags);
            out.push_str(" <span aria-hidden=\"true\">→</span><span class=\"sr\">moved to</span> ");
        }
        plain(out, &e.path, &mut self.flags);
        if unsafe_path(&e.path) || from.is_some_and(unsafe_path) {
            out.push_str("<span class=\"badge bad\">unsafe path</span>");
        }
        if [e.left.as_ref(), e.right.as_ref()]
            .iter()
            .any(|s| matches!(digest(*s), Digest::NotRead))
        {
            out.push_str("<span class=\"badge\">content not read</span>");
        }
        out.push_str("</td><td>");
        self.side(out, e.left.as_ref());
        out.push_str("</td><td>");
        self.side(out, e.right.as_ref());
        out.push_str("</td><td>");
        match (digest(e.left.as_ref()), digest(e.right.as_ref())) {
            (Digest::None, Digest::None) => {}
            (Digest::Sha(a), Digest::Sha(b)) if a == b => self.sha(out, a),
            (a, b) => {
                self.digest(out, &a);
                out.push_str(" <span aria-hidden=\"true\">→</span><span class=\"sr\">then</span> ");
                self.digest(out, &b);
            }
        }
        out.push_str("</td></tr>\n");
    }

    fn side(&mut self, out: &mut String, side: Option<&EntrySide>) {
        let Some(s) = side else {
            out.push_str("<span class=\"mute\" aria-hidden=\"true\">—</span><span class=\"sr\">absent</span>");
            return;
        };
        match s.entry_type {
            EntryType::File => match s.size {
                Some(n) => {
                    out.push_str("file, ");
                    out.push_str(&text::count(n, "byte", "bytes"));
                }
                None => out.push_str("file, size unknown"),
            },
            EntryType::Directory => out.push_str("directory"),
            EntryType::Symlink | EntryType::Hardlink => {
                out.push_str(if s.entry_type == EntryType::Symlink {
                    "symbolic link"
                } else {
                    "hard link"
                });
                out.push_str(
                    " <span aria-hidden=\"true\">→</span><span class=\"sr\">to</span> <code>",
                );
                plain(out, s.target.as_deref().unwrap_or(""), &mut self.flags);
                out.push_str("</code>");
            }
            EntryType::Other => out.push_str("special file"),
            EntryType::Unknown => out.push_str("unknown type"),
        }
    }

    fn digest(&mut self, out: &mut String, d: &Digest<'_>) {
        match d {
            Digest::None => out.push_str("<span class=\"mute\">—</span>"),
            Digest::NotRead => out.push_str("<span class=\"mute\">not read</span>"),
            Digest::Sha(h) => self.sha(out, h),
        }
    }

    /// The first 12 digits of a digest, with all of it as the tooltip.
    fn sha(&mut self, out: &mut String, hex: &str) {
        out.push_str("<code title=\"");
        push_escaped(out, &text::escape_controls(hex));
        out.push_str("\">");
        let short: String = hex.chars().take(12).collect();
        plain(out, &short, &mut self.flags);
        out.push_str("</code>");
    }

    /// The row under a modified entry: its child document in a collapsed
    /// `<details>`, or why it has none.
    fn child_row(
        &mut self,
        out: &mut String,
        parent: &DiffDocument,
        e: &TreeEntry,
        chain: &[&str],
        depth: u32,
        budget: usize,
    ) {
        let file = |s: Option<&EntrySide>| s.is_some_and(|s| s.entry_type == EntryType::File);
        let Some(child) = &e.child_document else {
            if !file(e.left.as_ref()) || !file(e.right.as_ref()) {
                return; // links: the targets are in the row
            }
            let read = !matches!(digest(e.left.as_ref()), Digest::NotRead)
                && !matches!(digest(e.right.as_ref()), Digest::NotRead);
            let why = if !read {
                "No child document: the content of this entry was not read."
            } else if parent.options.tree.child_budget == 0 {
                "No child document: child documents are turned off (<code>tree.childBudget</code> is 0)."
            } else {
                "No child document: it was left out because it exceeds size limits (<code>children-omitted</code>)."
            };
            let _ = writeln!(
                out,
                "<tr class=\"child\"><td></td><td colspan=\"4\"><p class=\"note\">{why}</p></td></tr>"
            );
            return;
        };
        let cd = DiffDocument::from(ChildDocument::clone(child));
        let kind = match cd.kind {
            Kind::Text => "Text",
            Kind::Binary => "Binary",
            Kind::Tree => "Archive",
            _ => "Document",
        };
        let _ = write!(
            out,
            "<tr class=\"child\"><td></td><td colspan=\"4\"><details>\n<summary>{kind}: "
        );
        plain(out, &summary::verdict(&cd), &mut self.flags);
        out.push_str("</summary>\n");
        if depth >= self.options.max_depth {
            self.notes.depth_cut = true;
            out.push_str(
                "<p class=\"note\">Not shown: deeper than this report's depth limit.</p>\n",
            );
        } else {
            if !cd.warnings.is_empty() {
                html::warnings_list(out, &cd.warnings);
            }
            let mut path = chain.to_vec();
            path.push(&e.path);
            match cd.kind {
                _ if cd.status == Status::Error => out.push_str(
                    "<p class=\"note\">No diff to show: the comparison ended with an error.</p>\n",
                ),
                Kind::Text => self.text_child(out, &cd, &path, budget),
                Kind::Binary => self.binary_child(out, &cd, &path, budget),
                Kind::Tree => self.entries(out, &cd, &path, depth + 1, budget),
                _ => out.push_str(
                    "<p class=\"note\">This kind of document cannot be shown here.</p>\n",
                ),
            }
        }
        out.push_str("</details></td></tr>\n");
    }

    /// Both contents of a child, from [`EntryContents`], within the content
    /// budget.
    fn fetch(&mut self, cd: &DiffDocument, path: &[&str]) -> Option<(Vec<u8>, Vec<u8>)> {
        let total = cd.left.size.saturating_add(cd.right.size);
        if total > self.content_left {
            self.notes.content_cut = true;
            return None;
        }
        self.content_left -= total;
        let got = self.contents.contents(path);
        if got.is_none() {
            self.notes.no_content = true;
        }
        got
    }

    fn text_child(&mut self, out: &mut String, cd: &DiffDocument, path: &[&str], budget: usize) {
        if cd.hunks.is_empty() {
            out.push_str("<p class=\"note\">No changes under the selected options.</p>\n");
            return;
        }
        match self.fetch(cd, path) {
            Some((l, r)) => {
                let ids = format!("c{}-", self.next_id);
                self.next_id += 1;
                match html::text_fragment(cd, &l, &r, &ids, self.options, budget, &mut self.flags) {
                    Ok((fragment, cut)) if !fragment.is_empty() => {
                        out.push_str(&fragment);
                        self.notes.truncated |= cut;
                        return;
                    }
                    Ok(_) => {
                        out.push_str("<p class=\"note\">No changes under the selected options.</p>\n");
                        return;
                    }
                    Err(_) => {
                        self.notes.mismatch = true;
                        out.push_str("<p class=\"note\">The supplied content does not match this entry, so the changes are listed by line number.</p>\n");
                    }
                }
            }
            None => out.push_str("<p class=\"note\">The content of this entry is not available to this report, so the changes are listed by line number.</p>\n"),
        }
        self.hunk_list(out, cd, budget);
    }

    fn hunk_list(&mut self, out: &mut String, cd: &DiffDocument, budget: usize) {
        let start = out.len();
        out.push_str("<ul class=\"hunks\">\n");
        for (i, h) in cd.hunks.iter().enumerate() {
            if out.len() - start > budget {
                let _ = writeln!(
                    out,
                    "<li>… {} not listed</li>",
                    text::count((cd.hunks.len() - i) as u64, "more hunk", "more hunks")
                );
                self.notes.truncated = true;
                break;
            }
            let (mut added, mut removed) = (0u64, 0u64);
            for op in &h.ops {
                if matches!(op.op, OpKind::Delete | OpKind::Replace) {
                    removed = removed.saturating_add(op.left_range.count);
                }
                if matches!(op.op, OpKind::Insert | OpKind::Replace) {
                    added = added.saturating_add(op.right_range.count);
                }
            }
            let _ = writeln!(
                out,
                "<li><code>@@ -{} +{} @@</code> {}, {added} added</li>",
                range_text(h.left),
                range_text(h.right),
                text::count(removed, "line removed", "lines removed")
            );
        }
        out.push_str("</ul>\n");
    }

    fn binary_child(&mut self, out: &mut String, cd: &DiffDocument, path: &[&str], budget: usize) {
        if hex::check_ranges(cd).is_err() {
            out.push_str("<p class=\"note\">The byte ranges of this entry do not fit its sizes and are not shown.</p>\n");
            return;
        }
        if cd.bytes.is_empty() {
            out.push_str("<p class=\"note\">No changes.</p>\n");
            return;
        }
        let contents = self.fetch(cd, path);
        let inputs = match &contents {
            Some((l, r)) if source::check_inputs(cd, source::Given::Bytes(l, r)).is_ok() => {
                Some((l.as_slice(), r.as_slice()))
            }
            Some(_) => {
                self.notes.mismatch = true;
                out.push_str("<p class=\"note\">The supplied content does not match this entry, so the ranges are listed without a hex dump.</p>\n");
                None
            }
            None => {
                out.push_str("<p class=\"note\">The content of this entry is not available to this report, so the ranges are listed without a hex dump.</p>\n");
                None
            }
        };
        if hex::ranges(out, cd, inputs, budget, &mut self.hex) {
            self.notes.truncated = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_paths_are_recognized() {
        for p in [
            "/etc/passwd",
            "../x",
            "a/../../b",
            "C:/Windows",
            "..",
            "\\x",
        ] {
            assert!(unsafe_path(p), "{p}");
        }
        for p in ["a/b", "..a/b", "a..", "c/C:", "x/.../y"] {
            assert!(!unsafe_path(p), "{p}");
        }
    }

    #[test]
    fn hunk_ranges_follow_gnu() {
        assert_eq!(range_text(Range { start: 1, count: 0 }), "0,0");
        assert_eq!(range_text(Range { start: 5, count: 0 }), "4,0");
        assert_eq!(range_text(Range { start: 3, count: 1 }), "3");
        assert_eq!(range_text(Range { start: 3, count: 4 }), "3,4");
    }
}
