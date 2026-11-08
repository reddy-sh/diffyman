//! Character classes and escaping shared by the renderers.

use std::borrow::Cow;
use std::fmt::Write as _;

/// A character that renders as nothing (or reorders text) and is therefore
/// shown as a visible marker in display output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Invisible {
    /// Short visible label, such as `RLO`.
    pub label: &'static str,
    /// A bidirectional control (Trojan Source, CVE-2021-42574).
    pub bidi: bool,
}

/// Classifies `c` as an invisible, control or bidi-control character.
///
/// Zero-width joiner and non-joiner are not listed: they are part of emoji
/// sequences and of several scripts, and they never reorder text.
pub(crate) fn invisible(c: char) -> Option<Invisible> {
    let bidi = |label| Some(Invisible { label, bidi: true });
    let hidden = |label| Some(Invisible { label, bidi: false });
    match c {
        '\t' => None,
        '\u{0}' => hidden("NUL"),
        '\u{1b}' => hidden("ESC"),
        '\u{7f}' => hidden("DEL"),
        '\u{ad}' => hidden("SHY"),
        '\u{34f}' => hidden("CGJ"),
        '\u{61c}' => bidi("ALM"),
        '\u{115f}' | '\u{1160}' | '\u{3164}' | '\u{ffa0}' => hidden("FILLER"),
        '\u{180e}' => hidden("MVS"),
        '\u{200b}' => hidden("ZWSP"),
        '\u{200e}' => bidi("LRM"),
        '\u{200f}' => bidi("RLM"),
        '\u{2028}' => hidden("LSEP"),
        '\u{2029}' => hidden("PSEP"),
        '\u{202a}' => bidi("LRE"),
        '\u{202b}' => bidi("RLE"),
        '\u{202c}' => bidi("PDF"),
        '\u{202d}' => bidi("LRO"),
        '\u{202e}' => bidi("RLO"),
        '\u{2060}' => hidden("WJ"),
        '\u{2066}' => bidi("LRI"),
        '\u{2067}' => bidi("RLI"),
        '\u{2068}' => bidi("FSI"),
        '\u{2069}' => bidi("PDI"),
        '\u{feff}' => hidden("ZWNBSP"),
        // Other C0 and C1 controls, Khmer inherent vowels, invisible math
        // operators, deprecated format characters, interlinear annotation,
        // musical format characters and tag characters.
        '\u{0}'..='\u{1f}'
        | '\u{80}'..='\u{9f}'
        | '\u{17b4}'
        | '\u{17b5}'
        | '\u{2061}'..='\u{2064}'
        | '\u{206a}'..='\u{206f}'
        | '\u{fff9}'..='\u{fffb}'
        | '\u{1d173}'..='\u{1d17a}'
        | '\u{e0000}'..='\u{e007f}' => hidden(""),
        _ => None,
    }
}

/// The visible label for an invisible character: its short name, or `U+XXXX`.
pub(crate) fn invisible_label(c: char, inv: Invisible) -> Cow<'static, str> {
    if inv.label.is_empty() {
        Cow::Owned(code_point(c))
    } else {
        Cow::Borrowed(inv.label)
    }
}

/// `U+XXXX` (at least four hex digits).
pub(crate) fn code_point(c: char) -> String {
    format!("U+{:04X}", u32::from(c))
}

/// Writes `s` with `&`, `<`, `>`, `"` and `'` escaped, for HTML text and
/// attribute values alike.
pub(crate) fn push_escaped(out: &mut String, s: &str) {
    let mut last = 0;
    for (i, b) in s.bytes().enumerate() {
        let rep = match b {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            b'\'' => "&#39;",
            _ => continue,
        };
        out.push_str(&s[last..i]);
        out.push_str(rep);
        last = i + 1;
    }
    out.push_str(&s[last..]);
}

/// Quotes a path the way git does when it holds characters that would break
/// a header line or a terminal: the name goes in double quotes, with `\"`,
/// `\\`, C escapes (`\t`, `\n`, ...) and octal escapes of the UTF-8 bytes of
/// every other control, bidi-control or invisible character. Other names are
/// returned unchanged.
pub(crate) fn quote_path(name: &str) -> Cow<'_, str> {
    let needs = |c: char| c == '"' || c == '\\' || c == '\t' || invisible(c).is_some();
    if !name.chars().any(needs) {
        return Cow::Borrowed(name);
    }
    let mut out = String::with_capacity(name.len() + 8);
    out.push('"');
    for c in name.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{b}' => out.push_str("\\v"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if invisible(c).is_some() => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    let _ = write!(out, "\\{b:03o}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    Cow::Owned(out)
}

/// Replaces control, bidi-control and invisible characters with `<LABEL>`
/// markers, for text shown on a terminal.
pub(crate) fn escape_controls(s: &str) -> Cow<'_, str> {
    if !s.chars().any(|c| invisible(c).is_some()) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match invisible(c) {
            Some(inv) => {
                out.push('<');
                out.push_str(&invisible_label(c, inv));
                out.push('>');
            }
            None => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// A similarity in basis points as a percentage without trailing zeros:
/// `72.73%`, `75%`, `100%`.
pub(crate) fn percent(bp: u32) -> String {
    let whole = bp / 100;
    let frac = bp % 100;
    if frac == 0 {
        format!("{whole}%")
    } else if frac.is_multiple_of(10) {
        format!("{whole}.{}%", frac / 10)
    } else {
        format!("{whole}.{frac:02}%")
    }
}

/// `n` followed by the singular or plural noun.
pub(crate) fn count(n: u64, singular: &str, plural: &str) -> String {
    format!("{n} {}", if n == 1 { singular } else { plural })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_specials() {
        let mut out = String::new();
        push_escaped(&mut out, r#"</script><img src=x onerror="a('b')">&"#);
        assert_eq!(
            out,
            "&lt;/script&gt;&lt;img src=x onerror=&quot;a(&#39;b&#39;)&quot;&gt;&amp;"
        );
    }

    #[test]
    fn quotes_paths_like_git() {
        assert_eq!(quote_path("plain name.txt"), "plain name.txt");
        assert_eq!(quote_path("é.txt"), "é.txt");
        assert_eq!(quote_path("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote_path("new\nline"), "\"new\\nline\"");
        assert_eq!(quote_path("tab\there"), "\"tab\\there\"");
        assert_eq!(quote_path("esc\u{1b}[31m"), "\"esc\\033[31m\"");
        assert_eq!(quote_path("rlo\u{202e}txt"), "\"rlo\\342\\200\\256txt\"");
    }

    #[test]
    fn marks_invisible_characters() {
        assert_eq!(invisible('\u{202e}').map(|i| i.bidi), Some(true));
        assert_eq!(invisible('\u{200b}').map(|i| i.bidi), Some(false));
        assert_eq!(invisible('\u{200d}'), None, "ZWJ is part of emoji");
        assert_eq!(invisible('a'), None);
        assert_eq!(invisible('\t'), None);
        assert_eq!(
            escape_controls("a\u{1b}[2Jb\u{202e}c\u{85}"),
            "a<ESC>[2Jb<RLO>c<U+0085>"
        );
        assert_eq!(escape_controls("plain"), "plain");
    }

    #[test]
    fn formats_percentages() {
        assert_eq!(percent(7273), "72.73%");
        assert_eq!(percent(7500), "75%");
        assert_eq!(percent(7250), "72.5%");
        assert_eq!(percent(10_000), "100%");
        assert_eq!(percent(0), "0%");
        assert_eq!(percent(1), "0.01%");
        assert_eq!(percent(5), "0.05%");
    }
}
