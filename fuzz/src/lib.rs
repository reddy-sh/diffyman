//! Shared pieces of the diffyman fuzz targets: input framing and the
//! invariant checks the targets assert.
//!
//! Every check panics on a violation, so libFuzzer records the input as a
//! crash. The rules come from `docs/spec/diff-document.md` (§5 canonical form
//! and `id`, §6 output shapes, §7 text procedure, §10 warnings) and
//! `docs/spec/text-granularity-and-moves.md` (§3 inline spans, §4 moves).

use diffyman::model::{
    self, DiffDocument, FileMeta, Hunk, Kind, LineTerminator, MoveKind, Op, OpKind, OpRef, Options,
    PositionEncoding, Severity, Side, Similarity, Span, Status, WhitespaceMode,
};
use diffyman::{Line, Terminator, decode_text, split_lines};
use diffyman_algo::key::{self, KeyBuilder, KeyOptions};

/// Splits `data` into `N` chunks. Every chunk but the last is a little-endian
/// `u16` length followed by that many bytes (cut short at the end of the
/// data); the last chunk is the rest. `seed_corpus.py` writes the same layout.
pub fn chunks<const N: usize>(data: &[u8]) -> [&[u8]; N] {
    let mut out = [&data[..0]; N];
    let mut rest = data;
    for slot in out.iter_mut().take(N.saturating_sub(1)) {
        let (len, after) = match rest {
            [a, b, tail @ ..] => (usize::from(u16::from_le_bytes([*a, *b])), tail),
            _ => (0, &rest[rest.len()..]),
        };
        let n = len.min(after.len());
        *slot = &after[..n];
        rest = &after[n..];
    }
    if let Some(last) = out.last_mut() {
        *last = rest;
    }
    out
}

/// The canonical JSON of a document without `provenance`: the form two runs
/// (and two bindings) must agree on byte for byte.
pub fn conformance_form(json: &str) -> String {
    let doc: DiffDocument = serde_json::from_str(json).expect("output reads as a DiffDocument");
    model::to_conformance_json(&doc).expect("conformance form serializes")
}

/// Converts an inline offset in `encoding` units to a byte offset into
/// `content`. `None` when the offset is past the end or not on a character
/// boundary (inside a UTF-8 sequence or a UTF-16 surrogate pair).
pub fn byte_offset(content: &str, offset: u64, encoding: PositionEncoding) -> Option<usize> {
    let target = usize::try_from(offset).ok()?;
    if encoding == PositionEncoding::Utf8 {
        return content.is_char_boundary(target).then_some(target);
    }
    let mut units = 0_usize;
    for (i, c) in content.char_indices() {
        if units == target {
            return Some(i);
        }
        if units > target {
            return None;
        }
        units += match encoding {
            PositionEncoding::Utf16 => c.len_utf16(),
            _ => 1,
        };
    }
    (units == target).then_some(content.len())
}

/// Checks that `lines` is exactly the line split of `text` (spec §7.2): the
/// lines tile the text, contents hold no CR or LF, CR followed by LF is one
/// CRLF terminator, and only the last line may lack a terminator (and then it
/// is not empty).
pub fn check_split(text: &str, lines: &[Line]) {
    let mut pos = 0;
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(
            line.start, pos,
            "line {i} does not start where the previous one ended"
        );
        assert!(
            line.start <= line.end && line.end <= text.len(),
            "line {i} out of bounds"
        );
        let content = line.content(text);
        assert!(
            !content.contains(['\r', '\n']),
            "line {i} content holds a terminator"
        );
        let end = line.end + line.terminator.len();
        assert!(end <= text.len(), "line {i} terminator out of bounds");
        assert_eq!(
            &text[line.end..end],
            line.terminator.as_str(),
            "line {i} terminator text"
        );
        if line.terminator == Terminator::Cr {
            assert!(
                !text[end..].starts_with('\n'),
                "CR LF split into two terminators at line {i}"
            );
        }
        if line.terminator.is_empty() {
            assert_eq!(i + 1, lines.len(), "unterminated line {i} is not the last");
            assert!(!content.is_empty(), "empty unterminated last line");
        }
        pos = end;
    }
    assert_eq!(pos, text.len(), "lines do not cover the text");
}

/// One decoded input: its text (after the BOM) and its lines.
struct Decoded<'a> {
    text: &'a str,
    lines: Vec<Line>,
}

impl Decoded<'_> {
    fn content(&self, line: usize) -> &str {
        self.lines[line].content(self.text)
    }

    fn len(&self) -> u64 {
        self.lines.len() as u64
    }
}

/// Checks a canonical JSON document returned by the engine for `left` and
/// `right` and returns it parsed. Asserts:
///
/// - the text is canonical JSON (re-canonicalizing gives the same bytes) and
///   reads back as a `DiffDocument` that writes the same bytes;
/// - `id` recomputes from the document;
/// - file meta matches the inputs, warnings are sorted, unique and carry
///   their fixed severity and message, and `status` follows them;
/// - for a text document: hunks and ops tile both sides inside the line
///   counts, `equal` and gap lines have equal comparison keys, context rules
///   hold, inline spans are in bounds, moves reference the right ops, and
///   `stats` adds up;
/// - for a binary document: the byte ranges are sorted, their gaps are equal
///   bytes, they rebuild the right input from the left one, and the byte
///   stats add up.
pub fn check_document(json: &str, left: &[u8], right: &[u8]) -> DiffDocument {
    let value: serde_json::Value = serde_json::from_str(json).expect("output is JSON");
    let again = model::jcs::canonicalize(&value).expect("output re-canonicalizes");
    assert_eq!(again, json, "output is not canonical JSON");
    let doc: DiffDocument = serde_json::from_str(json).expect("output reads as a DiffDocument");
    let written = model::to_canonical_json(&doc).expect("document re-serializes");
    assert_eq!(written, json, "read/write round trip changed the document");
    assert_eq!(
        model::compute_id(&doc).expect("id computes"),
        doc.id,
        "id does not recompute"
    );
    assert!(
        doc.id.strip_prefix("sha256:").is_some_and(|h| h.len() == 64
            && h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))),
        "malformed id {}",
        doc.id
    );
    assert_eq!(doc.schema_version, model::SCHEMA_VERSION);

    check_meta_bytes(&doc.left, left);
    check_meta_bytes(&doc.right, right);
    check_warnings(&doc);
    if doc.kind == Kind::Binary {
        check_binary(&doc, left, right);
        return doc;
    }
    if doc.kind != Kind::Text {
        // Other kinds have their own blocks; the generic checks above apply.
        return doc;
    }
    assert!(doc.bytes.is_empty(), "text document has byte ranges");
    let l = check_text_meta(&doc, Side::Left, left);
    let r = check_text_meta(&doc, Side::Right, right);
    if doc.status == Status::Error {
        assert!(doc.stats.is_none(), "error document has stats");
        assert!(
            doc.hunks.is_empty() && doc.moves.is_empty(),
            "error document has hunks or moves"
        );
        return doc;
    }
    let (Some(l), Some(r)) = (l, r) else {
        panic!("a side failed to decode but the status is not error");
    };
    check_text(&doc, &l, &r);
    doc
}

/// Spec §6.7, §6.3.1 and §17: the ranges are a complete edit script.
fn check_binary(doc: &DiffDocument, left: &[u8], right: &[u8]) {
    assert!(
        doc.hunks.is_empty() && doc.moves.is_empty(),
        "binary document has hunks or moves"
    );
    for meta in [&doc.left, &doc.right] {
        assert!(
            meta.line_count.is_none()
                && meta.line_terminator.is_none()
                && meta.final_newline.is_none(),
            "binary document has line meta"
        );
    }
    if doc.status == Status::Error {
        assert!(
            doc.stats.is_none() && doc.bytes.is_empty(),
            "error document has results"
        );
        return;
    }
    let as_usize = |v: u64| usize::try_from(v).expect("offset fits");
    let mut rebuilt = Vec::with_capacity(right.len());
    let (mut li, mut ri) = (0usize, 0usize);
    for (i, r) in doc.bytes.iter().enumerate() {
        let (lo, ll) = (as_usize(r.left_offset), as_usize(r.left_length));
        let (ro, rl) = (as_usize(r.right_offset), as_usize(r.right_length));
        let want = match (ll > 0, rl > 0) {
            (true, true) => OpKind::Replace,
            (true, false) => OpKind::Delete,
            (false, true) => OpKind::Insert,
            (false, false) => panic!("empty byte range {i}"),
        };
        assert_eq!(r.op, want, "op of byte range {i}");
        assert!(lo >= li && ro >= ri, "byte ranges out of order at {i}");
        assert_eq!(lo - li, ro - ri, "gap lengths differ before byte range {i}");
        if i > 0 {
            assert!(lo - li >= 8, "byte ranges closer than 8 bytes at {i}");
        }
        assert!(
            lo + ll <= left.len() && ro + rl <= right.len(),
            "byte range {i} out of bounds"
        );
        assert_eq!(
            left[li..lo],
            right[ri..ro],
            "gap before byte range {i} differs"
        );
        rebuilt.extend_from_slice(&left[li..lo]);
        rebuilt.extend_from_slice(&right[ro..ro + rl]);
        li = lo + ll;
        ri = ro + rl;
    }
    assert_eq!(
        left[li..],
        right[ri..],
        "tail after the byte ranges differs"
    );
    rebuilt.extend_from_slice(&left[li..]);
    assert!(
        rebuilt == right,
        "byte ranges do not rebuild the right input"
    );
    let stats = doc.stats.as_ref().expect("stats present");
    let bytes = stats.bytes.as_ref().expect("byte stats present");
    let removed: u64 = doc.bytes.iter().map(|r| r.left_length).sum();
    let added: u64 = doc.bytes.iter().map(|r| r.right_length).sum();
    assert_eq!(
        (
            bytes.left,
            bytes.right,
            bytes.added,
            bytes.removed,
            bytes.unchanged
        ),
        (
            left.len() as u64,
            right.len() as u64,
            added,
            removed,
            left.len() as u64 - removed
        ),
        "byte stats"
    );
    assert_eq!(stats.change_count, doc.bytes.len() as u64, "changeCount");
    assert_eq!(
        doc.status == Status::Identical,
        doc.bytes.is_empty(),
        "status"
    );
}

fn check_meta_bytes(meta: &FileMeta, bytes: &[u8]) {
    assert_eq!(meta.size, bytes.len() as u64, "size");
    assert_eq!(meta.sha256, model::sha256_hex(bytes), "sha256");
    assert_eq!(meta.bom, bytes.starts_with(b"\xEF\xBB\xBF"), "bom");
}

fn check_warnings(doc: &DiffDocument) {
    for w in &doc.warnings {
        assert_eq!(w.severity, w.code.severity(), "severity of {:?}", w.code);
        assert_eq!(w.message, w.code.message(), "message of {:?}", w.code);
    }
    for pair in doc.warnings.windows(2) {
        assert!(
            pair[0].sort_key() < pair[1].sort_key(),
            "warnings not sorted and unique: {:?}",
            doc.warnings
        );
    }
    let failed = doc.warnings.iter().any(|w| w.severity == Severity::Error);
    assert_eq!(
        failed,
        doc.status == Status::Error,
        "status {:?} vs warnings",
        doc.status
    );
}

fn has_warning(doc: &DiffDocument, code: &str, side: Option<Side>) -> bool {
    doc.warnings
        .iter()
        .any(|w| w.code.as_str() == code && w.side == side)
}

/// Checks the text fields of one side's meta and returns the decoded side.
fn check_text_meta<'a>(doc: &DiffDocument, side: Side, bytes: &'a [u8]) -> Option<Decoded<'a>> {
    let meta = if side == Side::Left {
        &doc.left
    } else {
        &doc.right
    };
    let Some(text) = decode_text(bytes) else {
        assert!(
            has_warning(doc, "invalid-utf8", Some(side)),
            "no invalid-utf8 for {side:?}"
        );
        assert!(
            meta.encoding.is_none() && meta.line_count.is_none(),
            "meta of undecodable side"
        );
        assert!(meta.line_terminator.is_none() && meta.final_newline.is_none());
        assert!(
            doc.warnings
                .iter()
                .all(|w| w.side != Some(side) || w.code.as_str() == "invalid-utf8"),
            "undecodable {side:?} side has other warnings"
        );
        return None;
    };
    assert!(
        !has_warning(doc, "invalid-utf8", Some(side)),
        "invalid-utf8 on valid UTF-8"
    );
    assert_eq!(
        has_warning(doc, "bom-stripped", Some(side)),
        meta.bom,
        "bom-stripped"
    );
    let lines = split_lines(text);
    check_split(text, &lines);
    assert_eq!(meta.encoding.as_deref(), Some("utf-8"));
    assert_eq!(meta.line_count, Some(lines.len() as u64), "lineCount");
    let final_newline = lines.last().is_some_and(|l| !l.terminator.is_empty());
    assert_eq!(meta.final_newline, Some(final_newline), "finalNewline");
    let unterminated_last = lines.last().is_some_and(|l| l.terminator.is_empty());
    assert_eq!(
        has_warning(doc, "no-newline-at-eof", Some(side)),
        unterminated_last
    );
    let mut styles: Vec<Terminator> = Vec::new();
    for line in &lines {
        if !line.terminator.is_empty() && !styles.contains(&line.terminator) {
            styles.push(line.terminator);
        }
    }
    let style = match styles.as_slice() {
        [] => LineTerminator::None,
        [Terminator::Lf] => LineTerminator::Lf,
        [Terminator::CrLf] => LineTerminator::Crlf,
        [Terminator::Cr] => LineTerminator::Cr,
        _ => LineTerminator::Mixed,
    };
    assert_eq!(meta.line_terminator, Some(style), "lineTerminator");
    assert_eq!(
        has_warning(doc, "mixed-line-endings", Some(side)),
        style == LineTerminator::Mixed
    );
    Some(Decoded { text, lines })
}

fn key_builder(options: &Options) -> KeyBuilder {
    KeyBuilder::new(&KeyOptions {
        whitespace: match options.ignore.whitespace {
            WhitespaceMode::Trailing => key::WhitespaceMode::Trailing,
            WhitespaceMode::Change => key::WhitespaceMode::Change,
            WhitespaceMode::All => key::WhitespaceMode::All,
            _ => key::WhitespaceMode::None,
        },
        ignore_case: options.ignore.case,
        regex: options.ignore.regex.clone(),
    })
    .expect("regex masks compile in a document without errors")
}

/// The 0-based line range `[start, end)` of a 1-based range, checked against
/// the side's line count.
fn span_of(range: model::Range, lines: u64, what: &str) -> (usize, usize) {
    assert!(range.start >= 1, "{what}: start {} < 1", range.start);
    assert!(
        range.start - 1 + range.count <= lines,
        "{what}: range {range:?} outside {lines} lines"
    );
    let start = usize::try_from(range.start - 1).expect("start fits");
    let count = usize::try_from(range.count).expect("count fits");
    (start, start + count)
}

fn same_key(kb: &KeyBuilder, l: &Decoded<'_>, li: usize, r: &Decoded<'_>, ri: usize) -> bool {
    kb.key(l.content(li), l.lines[li].terminator) == kb.key(r.content(ri), r.lines[ri].terminator)
}

/// Equal lines of one hunk: before its first change op, after its last one,
/// and the largest run between two of its change ops.
fn context_runs(hunk: &Hunk) -> (u64, u64, u64) {
    let changes: Vec<usize> = (0..hunk.ops.len())
        .filter(|&i| hunk.ops[i].op.is_change())
        .collect();
    let equal = |ops: &[Op]| -> u64 {
        ops.iter()
            .filter(|o| o.op == OpKind::Equal)
            .map(|o| o.left_range.count)
            .sum()
    };
    let (first, last) = (changes[0], changes[changes.len() - 1]);
    let inner = changes
        .windows(2)
        .map(|w| equal(&hunk.ops[w[0] + 1..w[1]]))
        .max()
        .unwrap_or(0);
    (
        equal(&hunk.ops[..first]),
        equal(&hunk.ops[last + 1..]),
        inner,
    )
}

fn check_text(doc: &DiffDocument, l: &Decoded<'_>, r: &Decoded<'_>) {
    let options = &doc.options;
    let kb = key_builder(options);
    let blank_lines = options.ignore.blank_lines;
    let context = u64::from(options.context);
    let (n_left, n_right) = (l.len(), r.len());

    // Hunks and ops.
    let (mut pos_l, mut pos_r) = (0_usize, 0_usize);
    let mut prev_trailing: Option<u64> = None;
    let (mut changes, mut added, mut removed) = (0_u64, 0_u64, 0_u64);
    let mut left_change = vec![false; l.lines.len()];
    let mut right_change = vec![false; r.lines.len()];
    for (h, hunk) in doc.hunks.iter().enumerate() {
        assert_eq!(hunk.id, format!("h{}", h + 1), "hunk id");
        assert!(!hunk.ops.is_empty(), "hunk {} has no ops", hunk.id);
        assert!(
            hunk.ops.iter().any(|o| o.op.is_change()),
            "hunk {} has no change op",
            hunk.id
        );
        let (hl, hl_end) = span_of(hunk.left, n_left, &hunk.id);
        let (hr, hr_end) = span_of(hunk.right, n_right, &hunk.id);
        assert!(
            hl >= pos_l && hr >= pos_r,
            "hunk {} overlaps the previous one",
            hunk.id
        );
        // Lines between hunks belong to equal (or ignored) ops.
        let (gap_l, gap_r) = (hl - pos_l, hr - pos_r);
        if !blank_lines {
            assert_eq!(gap_l, gap_r, "gap before {} differs per side", hunk.id);
            for k in 0..gap_l {
                assert!(
                    same_key(&kb, l, pos_l + k, r, pos_r + k),
                    "gap lines differ before {}",
                    hunk.id
                );
            }
        }
        let (leading, trailing, inner) = context_runs(hunk);
        assert!(
            leading <= context && trailing <= context,
            "context of {} too wide",
            hunk.id
        );
        assert!(
            inner <= 2 * context,
            "{} joins changes {inner} equal lines apart",
            hunk.id
        );
        if !blank_lines {
            // Hunks more than 2C equal lines apart each get C lines of
            // context, so a shorter context only happens at a file edge.
            if leading < context {
                assert!(hl == 0 && hr == 0, "{} leading context cut short", hunk.id);
            }
            if trailing < context {
                assert!(
                    hl_end == l.lines.len() && hr_end == r.lines.len(),
                    "{} trailing context cut short",
                    hunk.id
                );
            }
            if let Some(prev) = prev_trailing {
                assert!(
                    prev + gap_l as u64 + leading > 2 * context,
                    "{} should join the previous hunk",
                    hunk.id
                );
            }
        }
        let (mut ol, mut or) = (hl, hr);
        let mut prev_kind: Option<OpKind> = None;
        for (i, op) in hunk.ops.iter().enumerate() {
            let what = format!("{} op {i}", hunk.id);
            let (sl, el) = span_of(op.left_range, n_left, &what);
            let (sr, er) = span_of(op.right_range, n_right, &what);
            assert_eq!(
                (sl, sr),
                (ol, or),
                "{what} does not start where the previous op ended"
            );
            let (cl, cr) = (op.left_range.count, op.right_range.count);
            match op.op {
                OpKind::Equal => {
                    assert!(cl == cr && cl > 0, "{what}: equal counts {cl}/{cr}");
                    for k in 0..(el - sl) {
                        assert!(
                            same_key(&kb, l, sl + k, r, sr + k),
                            "{what}: equal lines differ"
                        );
                    }
                }
                OpKind::Insert => assert!(cl == 0 && cr > 0, "{what}: insert counts"),
                OpKind::Delete => assert!(cl > 0 && cr == 0, "{what}: delete counts"),
                OpKind::Replace => assert!(cl > 0 && cr > 0, "{what}: replace counts"),
                OpKind::Ignored => {
                    assert!(blank_lines, "{what}: ignored op without ignore.blankLines");
                    assert!(cl + cr > 0, "{what}: empty ignored op");
                    assert!(
                        (sl..el).all(|k| key::is_blank(l.content(k))),
                        "{what}: non-blank left"
                    );
                    assert!(
                        (sr..er).all(|k| key::is_blank(r.content(k))),
                        "{what}: non-blank right"
                    );
                }
                OpKind::Unknown => panic!("{what}: unknown op kind"),
            }
            if let Some(prev) = prev_kind {
                let joinable = |k: OpKind| k.is_change() || k == OpKind::Ignored;
                assert!(
                    !(joinable(prev) && joinable(op.op)),
                    "{what}: {prev:?} next to {:?}",
                    op.op
                );
            }
            prev_kind = Some(op.op);
            if op.op.is_change() {
                changes += 1;
                added += cr;
                removed += cl;
                left_change[sl..el].fill(true);
                right_change[sr..er].fill(true);
            }
            check_inline(doc, op, (sl, el), (sr, er), l, r, &what);
            (ol, or) = (el, er);
        }
        assert_eq!(
            (ol, or),
            (hl_end, hr_end),
            "ops of {} do not tile the hunk",
            hunk.id
        );
        (pos_l, pos_r) = (hl_end, hr_end);
        prev_trailing = Some(trailing);
    }
    if !blank_lines {
        let (gap_l, gap_r) = (l.lines.len() - pos_l, r.lines.len() - pos_r);
        assert_eq!(gap_l, gap_r, "lines after the last hunk differ per side");
        for k in 0..gap_l {
            assert!(
                same_key(&kb, l, pos_l + k, r, pos_r + k),
                "lines after the last hunk differ"
            );
        }
    }

    // Stats (spec §6.3).
    let stats = doc.stats.as_ref().expect("stats present");
    assert_eq!(
        (stats.lines_left, stats.lines_right),
        (n_left, n_right),
        "line counts"
    );
    assert_eq!(stats.hunk_count, doc.hunks.len() as u64, "hunkCount");
    assert_eq!(stats.change_count, changes, "changeCount");
    assert_eq!(
        (stats.lines_added, stats.lines_removed),
        (added, removed),
        "added/removed"
    );
    let ignored_left = n_left
        .checked_sub(stats.lines_unchanged + removed)
        .expect("unchanged + removed <= linesLeft");
    let ignored_right = n_right
        .checked_sub(stats.lines_unchanged + added)
        .expect("unchanged + added <= linesRight");
    if !blank_lines {
        assert_eq!(
            (ignored_left, ignored_right),
            (0, 0),
            "lines unaccounted for"
        );
    }
    let d = u128::from(n_left - ignored_left) + u128::from(n_right - ignored_right);
    let want = if d == 0 {
        Similarity::ONE
    } else {
        let n = (40_000 * u128::from(stats.lines_unchanged) + d) / (2 * d);
        Similarity::from_basis_points(u32::try_from(n).expect("similarity fits"))
    };
    assert_eq!(stats.similarity, want, "similarity");
    let want_status = if changes > 0 {
        Status::Different
    } else {
        Status::Identical
    };
    assert_eq!(doc.status, want_status, "status");
    if l.text == r.text {
        assert_eq!(
            doc.status,
            Status::Identical,
            "equal texts are not identical"
        );
    }
    let exact = options.ignore.whitespace == WhitespaceMode::None
        && !options.ignore.case
        && !blank_lines
        && options.ignore.regex.is_empty();
    if exact && doc.status == Status::Identical {
        assert_eq!(
            l.text, r.text,
            "identical under exact comparison but the texts differ"
        );
    }

    check_moves(doc, l, r, &left_change, &right_change);
}

fn check_inline(
    doc: &DiffDocument,
    op: &Op,
    (sl, el): (usize, usize),
    (sr, er): (usize, usize),
    l: &Decoded<'_>,
    r: &Decoded<'_>,
    what: &str,
) {
    if op.inline.is_empty() {
        return;
    }
    assert_eq!(op.op, OpKind::Replace, "{what}: spans on a {:?} op", op.op);
    assert_ne!(
        doc.options.granularity,
        model::Granularity::Line,
        "{what}: spans with granularity line"
    );
    let mut prev: Option<&Span> = None;
    for span in &op.inline {
        let (decoded, lo, hi, kind) = match span.side {
            Side::Left => (l, sl, el, OpKind::Delete),
            Side::Right => (r, sr, er, OpKind::Insert),
            Side::Unknown => panic!("{what}: span side unknown"),
        };
        assert_eq!(span.op, kind, "{what}: span op");
        let line = usize::try_from(span.line).expect("line fits");
        assert!(
            line > lo && line <= hi,
            "{what}: span line {line} outside the op"
        );
        let content = decoded.content(line - 1);
        let enc = doc.position_encoding;
        let start = byte_offset(content, span.start, enc);
        let end = byte_offset(content, span.end, enc);
        assert!(
            start.is_some() && end.is_some(),
            "{what}: span {span:?} not on character boundaries of {content:?}"
        );
        assert!(span.start < span.end, "{what}: empty span {span:?}");
        for m in &doc.moves {
            let range = if span.side == Side::Left {
                m.from.range
            } else {
                m.to.range
            };
            assert!(
                !(range.start..range.start + range.count).contains(&span.line),
                "{what}: span on moved line {}",
                span.line
            );
        }
        if let Some(p) = prev {
            let order = |s: &Span| (s.side == Side::Right, s.line, s.start);
            assert!(order(p) < order(span), "{what}: spans out of order");
            if p.side == span.side && p.line == span.line {
                assert!(p.end <= span.start, "{what}: spans overlap");
            }
        }
        prev = Some(span);
    }
}

fn check_moves(
    doc: &DiffDocument,
    l: &Decoded<'_>,
    r: &Decoded<'_>,
    left_change: &[bool],
    right_change: &[bool],
) {
    if !doc.options.detect_moves {
        assert!(doc.moves.is_empty(), "moves without detectMoves");
        return;
    }
    if has_warning(doc, "moves-skipped-limit", None) && doc.moves.is_empty() {
        return;
    }
    let mut left_moved = vec![false; l.lines.len()];
    let mut right_moved = vec![false; r.lines.len()];
    for (i, m) in doc.moves.iter().enumerate() {
        let what = format!("move {}", m.id);
        assert_eq!(m.id, format!("m{}", i + 1), "move id");
        assert_eq!(
            (m.from.side, m.to.side),
            (Side::Left, Side::Right),
            "{what}: sides"
        );
        assert!(
            m.from.range.count >= 1 && m.to.range.count >= 1,
            "{what}: empty range"
        );
        let (fl, fl_end) = span_of(m.from.range, l.len(), &what);
        let (tr, tr_end) = span_of(m.to.range, r.len(), &what);
        for k in fl..fl_end {
            assert!(left_change[k], "{what}: left line {} is not deleted", k + 1);
            assert!(!left_moved[k], "{what}: left line {} in two moves", k + 1);
            left_moved[k] = true;
        }
        for k in tr..tr_end {
            assert!(
                right_change[k],
                "{what}: right line {} is not inserted",
                k + 1
            );
            assert!(!right_moved[k], "{what}: right line {} in two moves", k + 1);
            right_moved[k] = true;
        }
        match m.kind {
            MoveKind::Pure => {
                assert_eq!(m.similarity, Similarity::ONE, "{what}: pure similarity");
                assert_eq!(m.from.range.count, m.to.range.count, "{what}: pure counts");
            }
            MoveKind::Edited => assert!(
                m.similarity.basis_points() >= diffyman_algo::moves::MOVE_EDITED_MIN_SIM_BP,
                "{what}: edited similarity {:?}",
                m.similarity
            ),
            MoveKind::Unknown => panic!("{what}: unknown kind"),
        }
        assert_eq!(
            m.from_ops,
            touching(doc, m.from.range, Side::Left),
            "{what}: fromOps"
        );
        assert_eq!(
            m.to_ops,
            touching(doc, m.to.range, Side::Right),
            "{what}: toOps"
        );
        assert!(
            !m.from_ops.is_empty() && !m.to_ops.is_empty(),
            "{what}: no ops"
        );
    }
    for pair in doc.moves.windows(2) {
        let key = |m: &model::Move| (m.from.range.start, m.to.range.start);
        assert!(key(&pair[0]) < key(&pair[1]), "moves out of order");
    }
}

/// Every op whose range on `side` shares a line with `range`, in document order.
fn touching(doc: &DiffDocument, range: model::Range, side: Side) -> Vec<OpRef> {
    let (start, end) = (range.start, range.start + range.count);
    let mut out = Vec::new();
    for hunk in &doc.hunks {
        for (i, op) in hunk.ops.iter().enumerate() {
            let r = if side == Side::Left {
                op.left_range
            } else {
                op.right_range
            };
            if r.count > 0 && r.start < end && start < r.start + r.count {
                out.push(OpRef {
                    hunk: hunk.id.clone(),
                    op: i as u64,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_split_by_length_prefix() {
        let data = [2, 0, b'a', b'b', 1, 0, b'c', b'd', b'e'];
        assert_eq!(chunks::<3>(&data), [&b"ab"[..], b"c", b"de"]);
        assert_eq!(chunks::<3>(&[9, 0, b'x']), [&b"x"[..], b"", b""]);
        assert_eq!(chunks::<2>(&[1]), [&b""[..], b""]);
        assert_eq!(chunks::<1>(b"all"), [&b"all"[..]]);
    }

    #[test]
    fn byte_offsets_per_encoding() {
        let s = "a\u{e9}\u{1f600}b";
        assert_eq!(byte_offset(s, 3, PositionEncoding::Utf8), Some(3));
        assert_eq!(byte_offset(s, 2, PositionEncoding::Utf8), None);
        assert_eq!(byte_offset(s, 2, PositionEncoding::Utf16), Some(3));
        assert_eq!(byte_offset(s, 3, PositionEncoding::Utf16), None);
        assert_eq!(byte_offset(s, 4, PositionEncoding::Utf16), Some(7));
        assert_eq!(byte_offset(s, 4, PositionEncoding::Codepoint), Some(8));
        assert_eq!(byte_offset(s, 5, PositionEncoding::Codepoint), None);
    }

    #[test]
    fn engine_output_passes_the_checks() {
        let (left, right) = (
            &b"a\nb\nc\nd\ne\nf\ng\nh\n"[..],
            &b"a\nB\nc\nd\ne\nf\ng\nH\n"[..],
        );
        for options in [
            "{}",
            r#"{"context": 0}"#,
            r#"{"granularity": "char", "positionEncoding": "utf-16"}"#,
            r#"{"detectMoves": true, "ignore": {"blankLines": true}}"#,
        ] {
            let json = diffyman::diff(left, right, options).unwrap();
            check_document(&json, left, right);
        }
    }

    /// A valid document for `a b c d` vs `a B c D` (one hunk: equal, replace,
    /// equal, replace), changed by `edit` and given a matching `id`, so only
    /// the edited rule can fail.
    fn tampered(edit: impl FnOnce(&mut DiffDocument)) {
        let (left, right) = (&b"a\nb\nc\nd\n"[..], &b"a\nB\nc\nD\n"[..]);
        let json = diffyman::diff(left, right, "{}").unwrap();
        let mut doc: DiffDocument = serde_json::from_str(&json).unwrap();
        edit(&mut doc);
        doc.id = model::compute_id(&doc).unwrap();
        check_document(&model::to_canonical_json(&doc).unwrap(), left, right);
    }

    #[test]
    #[should_panic(expected = "added/removed")]
    fn wrong_stats_are_caught() {
        tampered(|d| d.stats.as_mut().unwrap().lines_added += 1);
    }

    #[test]
    #[should_panic(expected = "does not start where the previous op ended")]
    fn ops_that_do_not_tile_are_caught() {
        tampered(|d| d.hunks[0].ops[1].left_range.start += 1);
    }

    #[test]
    #[should_panic(expected = "equal lines differ")]
    fn equal_ops_over_different_lines_are_caught() {
        tampered(|d| d.hunks[0].ops[1].op = OpKind::Equal);
    }

    #[test]
    #[should_panic(expected = "id does not recompute")]
    fn a_stale_id_is_caught() {
        let json = diffyman::diff(b"a\n", b"b\n", "{}").unwrap();
        let mut doc: DiffDocument = serde_json::from_str(&json).unwrap();
        doc.left.name = Some("x".to_owned());
        check_document(&model::to_canonical_json(&doc).unwrap(), b"a\n", b"b\n");
    }
}
