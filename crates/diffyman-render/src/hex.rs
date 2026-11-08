//! HTML for kind `binary`: the changed byte ranges (spec §6.7) with a
//! bounded hex dump of both sides of each range.

use std::fmt::Write as _;

use diffyman_model::{DiffDocument, FileMeta, OpKind, Status};

use crate::RenderError;
use crate::html::{self, Flags, HtmlOptions, plain};
use crate::source;
use crate::text::{self, push_escaped};

/// Bytes per hex dump row.
const ROW: u64 = 16;
/// Rows of one side of one range in a hex dump (256 bytes).
const MAX_ROWS: u64 = 16;

/// What is left of [`HtmlOptions::max_hex_bytes`] for one report.
#[derive(Debug)]
pub(crate) struct HexBudget {
    pub left: u64,
    /// A dump was cut short or left out because the budget ran out.
    pub cut: bool,
}

impl HexBudget {
    pub fn new(options: &HtmlOptions) -> HexBudget {
        HexBudget {
            left: options.max_hex_bytes as u64,
            cut: false,
        }
    }
}

/// Styles of the byte-range table and the hex dump.
pub(crate) const CSS: &str = r"table.ranges,table.entries{width:100%;border-collapse:collapse;font-size:13px;line-height:1.45;border:1px solid var(--bd);margin:.25rem 0 .75rem}
.ranges th,.entries th{background:var(--hh);text-align:left;font-weight:600;padding:.25rem .5rem;border-bottom:1px solid var(--bd)}
.ranges td,.entries td{padding:.2rem .5rem;vertical-align:top;border-top:1px solid var(--bd);overflow-wrap:anywhere}
.ranges .n{color:var(--mute);text-align:right;width:3em}
.ranges .op,.entries .st{white-space:nowrap}
.loc{display:block;font-size:.85rem;color:var(--mute)}
code.hex{display:block;white-space:pre;overflow-x:auto;font-size:12px;line-height:1.4;margin-top:.2rem}
.ho{color:var(--mute)}
.key.r{background:var(--mv-bg);color:var(--mv)}
tr.cut td{background:var(--inv-bg);color:var(--inv-fg);text-align:center;padding:.4rem}
";

/// Checks that every byte range lies inside its inputs (by the file meta)
/// and has the shape of spec §6.7.
pub(crate) fn check_ranges(doc: &DiffDocument) -> Result<(), RenderError> {
    let inside =
        |offset: u64, length: u64, size: u64| offset.checked_add(length).is_some_and(|e| e <= size);
    for r in &doc.bytes {
        if !inside(r.left_offset, r.left_length, doc.left.size)
            || !inside(r.right_offset, r.right_length, doc.right.size)
        {
            return Err(RenderError::InvalidDocument(
                "a byte range lies outside the input".to_owned(),
            ));
        }
        let shape_ok = match r.op {
            OpKind::Insert => r.left_length == 0 && r.right_length > 0,
            OpKind::Delete => r.left_length > 0 && r.right_length == 0,
            OpKind::Replace => r.left_length > 0 && r.right_length > 0,
            _ => false,
        };
        if !shape_ok {
            return Err(RenderError::InvalidDocument(
                "a byte range has an unknown op or the wrong lengths".to_owned(),
            ));
        }
    }
    Ok(())
}

/// The page of a binary document.
pub(crate) fn page(
    doc: &DiffDocument,
    given: source::Given<'_>,
    options: &HtmlOptions,
) -> Result<String, RenderError> {
    source::check_inputs(doc, given)?;
    let (left, right) = given.bytes();
    check_ranges(doc)?;
    let mut flags = Flags::default();
    let mut hex = HexBudget::new(options);
    let budget = options.max_bytes.saturating_sub(html::HEADER_RESERVE);
    let mut main = String::new();
    let mut truncated = false;
    if doc.status == Status::Error || doc.bytes.is_empty() {
        main.push_str(html::empty_notice(doc.status));
    } else {
        main.push_str("<h2>Changed byte ranges</h2>\n");
        truncated = ranges(&mut main, doc, Some((left, right)), budget, &mut hex);
    }
    Ok(html::assemble(
        doc,
        options,
        CSS,
        &mut flags,
        &main,
        |out, flags| header(out, doc, flags, truncated, hex.cut, options),
    ))
}

fn header(
    out: &mut String,
    doc: &DiffDocument,
    flags: &mut Flags,
    truncated: bool,
    hex_cut: bool,
    options: &HtmlOptions,
) {
    html::files_line(out, doc, flags);
    match doc.status {
        Status::Identical => out.push_str("<p class=\"status st-identical\"><strong>Identical.</strong> The files are byte for byte the same.</p>\n"),
        Status::Error => out.push_str(html::ERROR_STATUS),
        _ => {
            out.push_str("<p class=\"status st-different\"><strong>Different.</strong> ");
            out.push_str(&counts(doc));
            out.push_str("</p>\n");
        }
    }
    if truncated {
        out.push_str("<p class=\"notice\" role=\"note\"><strong>Truncated.</strong> This report reached its size limit; not every byte range is listed.</p>\n");
    }
    if hex_cut {
        hex_notice(out, options);
    }
    html::char_notices(out, flags);
    out.push_str(LEGEND);
    out.push_str("<details>\n<summary>Files and options</summary>\n");
    files_table(out, doc, flags);
    out.push_str("<p>Options: adapter ");
    push_escaped(out, adapter_name(doc));
    out.push_str(".</p>\n</details>\n");
    html::warnings_section(out, &doc.warnings);
}

/// `N changed ranges: A bytes added, R removed, U unchanged; S similar.`
pub(crate) fn counts(doc: &DiffDocument) -> String {
    let Some(stats) = &doc.stats else {
        return String::new();
    };
    let mut out = text::count(stats.change_count, "changed range", "changed ranges");
    if let Some(b) = &stats.bytes {
        let _ = write!(
            out,
            ": {} added, {} removed, {} unchanged",
            text::count(b.added, "byte", "bytes"),
            b.removed,
            b.unchanged
        );
    }
    let _ = write!(
        out,
        "; {} similar.",
        text::percent(stats.similarity.basis_points())
    );
    out
}

pub(crate) fn hex_notice(out: &mut String, options: &HtmlOptions) {
    let _ = writeln!(
        out,
        "<p class=\"notice\" role=\"note\"><strong>Hex dumps cut short.</strong> This report shows at most {} of hex dump; later byte ranges are listed without one.</p>",
        text::count(options.max_hex_bytes as u64, "byte", "bytes")
    );
}

pub(crate) const LEGEND: &str = concat!(
    "<ul class=\"legend\" aria-label=\"Legend\">",
    "<li><span class=\"key d\">−</span> deleted bytes</li>",
    "<li><span class=\"key a\">+</span> inserted bytes</li>",
    "<li><span class=\"key r\">±</span> replaced bytes</li>",
    "<li><del>4f</del> <ins>50</ins> changed bytes in a hex dump</li>",
    "<li><code>.</code> a byte outside printable ASCII in the text column</li>",
    "</ul>\n"
);

fn adapter_name(doc: &DiffDocument) -> &'static str {
    match doc.options.adapter {
        diffyman_model::Adapter::Auto => "auto",
        diffyman_model::Adapter::Text => "text",
        diffyman_model::Adapter::Binary => "binary",
        diffyman_model::Adapter::Tree => "tree",
        diffyman_model::Adapter::Unknown => "unknown",
    }
}

/// Name, media type, size, encoding and SHA-256 of both inputs.
fn files_table(out: &mut String, doc: &DiffDocument, flags: &mut Flags) {
    type Row = (&'static str, fn(&FileMeta) -> String);
    let (l, r) = (&doc.left, &doc.right);
    out.push_str("<table class=\"meta\">\n<thead><tr><td></td><th scope=\"col\">Left</th><th scope=\"col\">Right</th></tr></thead>\n<tbody>\n");
    out.push_str("<tr><th scope=\"row\">Name</th><td>");
    plain(out, l.name.as_deref().unwrap_or("(none)"), flags);
    out.push_str("</td><td>");
    plain(out, r.name.as_deref().unwrap_or("(none)"), flags);
    out.push_str("</td></tr>\n");
    let rows: [Row; 4] = [
        ("Media type", |m| {
            m.media_type.clone().unwrap_or_else(|| "unknown".to_owned())
        }),
        ("Size", |m| text::count(m.size, "byte", "bytes")),
        ("Encoding", |m| {
            m.encoding.clone().unwrap_or_else(|| "binary".to_owned())
        }),
        ("SHA-256", |m| m.sha256.clone()),
    ];
    for (name, f) in rows {
        let code = matches!(name, "Media type" | "SHA-256");
        let _ = write!(out, "<tr><th scope=\"row\">{name}</th>");
        for meta in [l, r] {
            out.push_str(if code { "<td><code>" } else { "<td>" });
            plain(out, &f(meta), flags);
            out.push_str(if code { "</code></td>" } else { "</td>" });
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n");
}

/// The table of the document's byte ranges, with hex dumps when `inputs`
/// holds both inputs (already checked against the document and
/// [`check_ranges`]). Rows stop once the table passes `budget` bytes.
/// Returns whether rows were left out.
pub(crate) fn ranges(
    out: &mut String,
    doc: &DiffDocument,
    inputs: Option<(&[u8], &[u8])>,
    budget: usize,
    hex: &mut HexBudget,
) -> bool {
    let start = out.len();
    out.push_str("<table class=\"ranges\">\n<thead><tr><th scope=\"col\">#</th><th scope=\"col\">Change</th><th scope=\"col\">Left</th><th scope=\"col\">Right</th></tr></thead>\n<tbody>\n");
    let mut shown = 0usize;
    let mut cut = false;
    for (i, r) in doc.bytes.iter().enumerate() {
        if out.len() - start > budget {
            cut = true;
            break;
        }
        let (class, icon, label) = match r.op {
            OpKind::Insert => ("a", "+", "inserted"),
            OpKind::Delete => ("d", "−", "deleted"),
            _ => ("r", "±", "replaced"),
        };
        let _ = write!(
            out,
            "<tr><td class=\"n\">{}</td><td class=\"op\"><span class=\"key {class}\" aria-hidden=\"true\">{icon}</span> {label}</td>",
            i + 1
        );
        side_cell(
            out,
            r.left_offset,
            r.left_length,
            inputs.map(|i| i.0),
            "del",
            hex,
        );
        side_cell(
            out,
            r.right_offset,
            r.right_length,
            inputs.map(|i| i.1),
            "ins",
            hex,
        );
        out.push_str("</tr>\n");
        shown += 1;
    }
    if cut {
        let _ = writeln!(
            out,
            "<tr class=\"cut\"><td colspan=\"4\" role=\"note\">Truncated: the report reached its size limit. {shown} of {} byte ranges are listed; use the JSON document for the rest.</td></tr>",
            doc.bytes.len()
        );
    }
    out.push_str("</tbody>\n</table>\n");
    cut
}

/// `0x1f (31)`.
fn offset_text(offset: u64) -> String {
    format!("0x{offset:x} ({offset})")
}

fn side_cell(
    out: &mut String,
    offset: u64,
    length: u64,
    bytes: Option<&[u8]>,
    tag: &str,
    hex: &mut HexBudget,
) {
    out.push_str("<td>");
    if length == 0 {
        let _ = write!(
            out,
            "<span class=\"loc\">no bytes; insertion point at offset {}</span>",
            offset_text(offset)
        );
    } else {
        let _ = write!(
            out,
            "<span class=\"loc\">{} at offset {}</span>",
            text::count(length, "byte", "bytes"),
            offset_text(offset)
        );
        if let Some(bytes) = bytes {
            dump(out, bytes, offset, length, tag, hex);
        }
    }
    out.push_str("</td>");
}

/// The hex dump of `[offset, offset + length)` of `bytes`: whole rows of 16
/// bytes aligned to multiples of 16, the range's bytes marked with `tag`,
/// at most [`MAX_ROWS`] rows and only what is left of the hex budget.
fn dump(out: &mut String, bytes: &[u8], offset: u64, length: u64, tag: &str, hex: &mut HexBudget) {
    let end = offset.saturating_add(length).min(bytes.len() as u64);
    if offset >= end {
        return;
    }
    let first = offset / ROW * ROW;
    let rows = (end - first).div_ceil(ROW).min(MAX_ROWS);
    let affordable = hex.left / ROW;
    let shown = rows.min(affordable);
    if shown < rows {
        hex.cut = true;
    }
    if shown == 0 {
        out.push_str(
            "<span class=\"cl\">Hex dump not shown: the report's hex limit was reached.</span>",
        );
        return;
    }
    hex.left -= shown * ROW;
    out.push_str("<code class=\"hex\">");
    for row in 0..shown {
        if row > 0 {
            out.push('\n');
        }
        hex_row(out, bytes, first + row * ROW, offset, end, tag);
    }
    out.push_str("</code>");
    let shown_end = (first + shown * ROW).min(end);
    if shown_end < end {
        let _ = write!(
            out,
            "<span class=\"cl\">… {} of this range not shown</span>",
            text::count(end - shown_end, "more byte", "more bytes")
        );
    }
}

/// One row: offset, 16 hex bytes (a gap after 8) and the printable ASCII
/// of those bytes, with the bytes in `[start, end)` inside `tag`.
fn hex_row(out: &mut String, bytes: &[u8], at: u64, start: u64, end: u64, tag: &str) {
    let byte = |p: u64| usize::try_from(p).ok().and_then(|i| bytes.get(i)).copied();
    let marked = |p: u64| byte(p).is_some() && p >= start && p < end;
    let _ = write!(out, "<span class=\"ho\">{at:08x}</span>  ");
    let mut open = false;
    for k in 0..ROW {
        let p = at + k;
        let m = marked(p);
        if k > 0 {
            if open && !m {
                let _ = write!(out, "</{tag}>");
                open = false;
            }
            out.push_str(if k == 8 { "  " } else { " " });
        }
        if m && !open {
            let _ = write!(out, "<{tag}>");
            open = true;
        }
        match byte(p) {
            Some(b) => {
                let _ = write!(out, "{b:02x}");
            }
            None => out.push_str("  "),
        }
    }
    if open {
        let _ = write!(out, "</{tag}>");
        open = false;
    }
    out.push_str("  |");
    for k in 0..ROW {
        let p = at + k;
        let m = marked(p);
        if m != open {
            let _ = if m {
                write!(out, "<{tag}>")
            } else {
                write!(out, "</{tag}>")
            };
            open = m;
        }
        match byte(p) {
            Some(b'&') => out.push_str("&amp;"),
            Some(b'<') => out.push_str("&lt;"),
            Some(b'>') => out.push_str("&gt;"),
            Some(b'"') => out.push_str("&quot;"),
            Some(b'\'') => out.push_str("&#39;"),
            Some(b @ 0x20..=0x7e) => out.push(char::from(b)),
            Some(_) => out.push('.'),
            None => out.push(' '),
        }
    }
    if open {
        let _ = write!(out, "</{tag}>");
    }
    out.push('|');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_mark_the_range_and_escape_ascii() {
        let bytes = b"<a href='x'>&\x00\x01\xffzz\n";
        let mut out = String::new();
        hex_row(&mut out, bytes, 0, 1, 3, "ins");
        assert_eq!(
            out,
            "<span class=\"ho\">00000000</span>  3c <ins>61 20</ins> 68 72 65 66 3d  27 78 27 3e 26 00 01 ff  |&lt;<ins>a </ins>href=&#39;x&#39;&gt;&amp;...|"
        );
        let mut out = String::new();
        hex_row(&mut out, bytes, 16, 17, 99, "del");
        // 13 missing bytes: 13 × 2 hex digits, 12 single gaps, one double
        // gap and the two spaces before the text column.
        assert_eq!(
            out,
            format!(
                "<span class=\"ho\">00000010</span>  7a <del>7a 0a</del>{}|z<del>z.</del>{}|",
                " ".repeat(42),
                " ".repeat(13)
            )
        );
    }

    #[test]
    fn dumps_are_bounded() {
        let bytes = vec![0u8; 4096];
        let mut hex = HexBudget {
            left: 1000,
            cut: false,
        };
        let mut out = String::new();
        dump(&mut out, &bytes, 10, 4000, "del", &mut hex);
        assert_eq!(out.matches("<span class=\"ho\">").count(), 16);
        // Rows 0x00 to 0xf0 cover the range up to byte 256 of 4010.
        assert!(
            out.contains("… 3754 more bytes of this range not shown"),
            "{out}"
        );
        assert_eq!(hex.left, 1000 - 256);
        assert!(!hex.cut);
        // 100 bytes left: six whole rows.
        hex.left = 100;
        let mut out = String::new();
        dump(&mut out, &bytes, 0, 4000, "del", &mut hex);
        assert_eq!(out.matches("<span class=\"ho\">").count(), 6);
        assert_eq!(hex.left, 4);
        assert!(hex.cut);
        let mut out = String::new();
        dump(&mut out, &bytes, 0, 1, "del", &mut hex);
        assert!(out.contains("hex limit was reached"));
    }
}
