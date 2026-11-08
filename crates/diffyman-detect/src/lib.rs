//! Type detection for diffyman (`detect.sniff@1`, Diff Document spec §16.1).
//!
//! [`detect`] decides whether one input is text or binary and which media type
//! it has. It looks only at the bytes and at the final component of the input
//! name; nothing depends on the platform, the locale or the file system.
//!
//! - **Class.** An input is [`Class::Empty`] when it has no bytes. It is
//!   [`Class::Text`] when it is valid UTF-8 (after an optional UTF-8 BOM), or
//!   valid UTF-16 after a UTF-16 BOM, and its first [`SNIFF_BYTES`] bytes hold
//!   no NUL and at most one control character in [`CONTROL_RATIO`] bytes.
//!   Everything else is [`Class::Binary`].
//! - **Media type.** For a binary input: the magic-number match of the pinned
//!   `infer` crate over the first [`MAGIC_BYTES`] bytes, else the extension
//!   table ([`EXTENSIONS`], binary entries), else `application/octet-stream`.
//!   For a text input: the extension table (text entries), else an XML or HTML
//!   start (`infer`'s WHATWG sniffing rules), else `text/plain`. An empty input
//!   takes any extension entry, else `text/plain`.
//!
//! The class never depends on the name, so renaming a file can change its
//! media type but never how it is compared.

/// Detection algorithm ID, recorded in provenance. Any change to the rules
/// above, to [`EXTENSIONS`] or to the pinned `infer` version bumps it.
pub const DETECT_ID: &str = "detect.sniff@1";

/// The pinned version of the magic-number library.
pub const INFER_VERSION: &str = "0.22.0";

/// Bytes at the start of an input that are checked for NUL and control
/// characters (git's binary rule looks at the same 8000 bytes).
pub const SNIFF_BYTES: usize = 8000;

/// Bytes at the start of an input that the magic-number matchers see.
pub const MAGIC_BYTES: usize = 65_536;

/// A text input may have at most one control character per this many bytes
/// of its sniff window (more than 10% makes it binary).
pub const CONTROL_RATIO: usize = 10;

/// Media type of a binary input that nothing recognizes.
pub const OCTET_STREAM: &str = "application/octet-stream";

/// Media type of text that nothing more specific recognizes.
pub const TEXT_PLAIN: &str = "text/plain";

/// Text or binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// No bytes at all. Compatible with both text and binary.
    Empty,
    /// UTF-8 or UTF-16 text.
    Text,
    /// Anything else.
    Binary,
}

/// A text encoding the detector recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// UTF-8, with or without a BOM.
    Utf8,
    /// UTF-16 little endian, after the BOM FF FE.
    Utf16Le,
    /// UTF-16 big endian, after the BOM FE FF.
    Utf16Be,
}

impl Encoding {
    /// The name written to the Diff Document's `encoding` field.
    pub fn as_str(self) -> &'static str {
        match self {
            Encoding::Utf8 => "utf-8",
            Encoding::Utf16Le => "utf-16le",
            Encoding::Utf16Be => "utf-16be",
        }
    }
}

/// The result of [`detect`] for one input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detection<'a> {
    /// Text, binary or empty.
    pub class: Class,
    /// The text encoding of a text or empty input; `None` for binary.
    pub encoding: Option<Encoding>,
    /// The media type, such as `text/plain` or `image/png`.
    pub media_type: &'static str,
    /// The input decoded as UTF-8 after an optional UTF-8 BOM, whatever the
    /// class: a binary input that is valid UTF-8 (for example one with NUL
    /// bytes) still has it, so a caller that forces text need not decode again.
    pub utf8: Option<&'a str>,
}

/// Which inputs an extension entry applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionClass {
    /// Applies to text and empty inputs.
    Text,
    /// Applies to binary and empty inputs.
    Binary,
}

/// The extension table: lowercase extension, the inputs it applies to, and
/// its media type. Only registered media types are listed; an extension that
/// is not here gives `text/plain` or `application/octet-stream`.
pub const EXTENSIONS: &[(&str, ExtensionClass, &str)] = &[
    ("css", ExtensionClass::Text, "text/css"),
    ("csv", ExtensionClass::Text, "text/csv"),
    ("htm", ExtensionClass::Text, "text/html"),
    ("html", ExtensionClass::Text, "text/html"),
    ("js", ExtensionClass::Text, "text/javascript"),
    ("json", ExtensionClass::Text, "application/json"),
    ("log", ExtensionClass::Text, "text/plain"),
    ("markdown", ExtensionClass::Text, "text/markdown"),
    ("md", ExtensionClass::Text, "text/markdown"),
    ("mjs", ExtensionClass::Text, "text/javascript"),
    ("svg", ExtensionClass::Text, "image/svg+xml"),
    ("tsv", ExtensionClass::Text, "text/tab-separated-values"),
    ("txt", ExtensionClass::Text, "text/plain"),
    ("xml", ExtensionClass::Text, "application/xml"),
    ("yaml", ExtensionClass::Text, "application/yaml"),
    ("yml", ExtensionClass::Text, "application/yaml"),
    ("bin", ExtensionClass::Binary, OCTET_STREAM),
    ("gif", ExtensionClass::Binary, "image/gif"),
    ("gz", ExtensionClass::Binary, "application/gzip"),
    ("jpeg", ExtensionClass::Binary, "image/jpeg"),
    ("jpg", ExtensionClass::Binary, "image/jpeg"),
    ("pdf", ExtensionClass::Binary, "application/pdf"),
    ("png", ExtensionClass::Binary, "image/png"),
    ("wasm", ExtensionClass::Binary, "application/wasm"),
    ("webp", ExtensionClass::Binary, "image/webp"),
    ("zip", ExtensionClass::Binary, "application/zip"),
];

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Detects the class, encoding and media type of one input. `name` is the
/// input name (only its final path component is used), or `None`.
pub fn detect<'a>(bytes: &'a [u8], name: Option<&str>) -> Detection<'a> {
    let utf8 = std::str::from_utf8(bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes)).ok();
    let window = &bytes[..bytes.len().min(SNIFF_BYTES)];
    let (class, encoding) = if bytes.is_empty() {
        (Class::Empty, Some(Encoding::Utf8))
    } else if utf8.is_some() {
        if clean_window(window) {
            (Class::Text, Some(Encoding::Utf8))
        } else {
            (Class::Binary, None)
        }
    } else if let Some(encoding) = utf16(bytes) {
        (Class::Text, Some(encoding))
    } else {
        (Class::Binary, None)
    };
    Detection {
        class,
        encoding,
        media_type: media_type(bytes, utf8, class, name),
        utf8,
    }
}

impl<'a> Detection<'a> {
    /// The same detection for the same `bytes` under another name: only the
    /// media type is computed again (the class never depends on the name).
    #[must_use]
    pub fn renamed(self, bytes: &'a [u8], name: Option<&str>) -> Detection<'a> {
        Detection {
            media_type: media_type(bytes, self.utf8, self.class, name),
            ..self
        }
    }
}

fn media_type(bytes: &[u8], utf8: Option<&str>, class: Class, name: Option<&str>) -> &'static str {
    let extension = name.and_then(extension_entry);
    match class {
        Class::Empty => extension.map_or(TEXT_PLAIN, |(_, media)| media),
        Class::Text => extension
            .filter(|(c, _)| *c == ExtensionClass::Text)
            .map(|(_, media)| media)
            .or_else(|| text_sniff(utf8.map(str::as_bytes)))
            .unwrap_or(TEXT_PLAIN),
        Class::Binary => infer::get(&bytes[..bytes.len().min(MAGIC_BYTES)])
            .map(|t| t.mime_type())
            .or_else(|| {
                extension
                    .filter(|(c, _)| *c == ExtensionClass::Binary)
                    .map(|(_, media)| media)
            })
            .unwrap_or(OCTET_STREAM),
    }
}

/// True when the window has no NUL and few control characters. TAB, LF, VT,
/// FF, CR and ESC (ANSI escapes in logs) are not counted.
fn clean_window(window: &[u8]) -> bool {
    if window.contains(&0) {
        return false;
    }
    let controls = window
        .iter()
        .filter(|&&b| matches!(b, 0x01..=0x08 | 0x0E..=0x1A | 0x1C..=0x1F | 0x7F))
        .count();
    controls * CONTROL_RATIO <= window.len()
}

/// The UTF-16 encoding of `bytes` when it starts with a UTF-16 BOM, the rest
/// is valid UTF-16 of even length, and the sniff window holds no U+0000 and
/// few control characters (so UTF-32 with its `FF FE 00 00` BOM is not text).
fn utf16(bytes: &[u8]) -> Option<Encoding> {
    let (encoding, body) = match bytes.strip_prefix(b"\xFF\xFE") {
        Some(body) => (Encoding::Utf16Le, body),
        None => (Encoding::Utf16Be, bytes.strip_prefix(b"\xFE\xFF")?),
    };
    let (pairs, rest) = body.as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    let units = pairs.iter().map(|&pair| match encoding {
        Encoding::Utf16Be => u16::from_be_bytes(pair),
        _ => u16::from_le_bytes(pair),
    });
    let window_units = SNIFF_BYTES / 2;
    let mut controls = 0usize;
    let mut seen = 0usize;
    for (i, ch) in char::decode_utf16(units).enumerate() {
        let ch = ch.ok()?;
        if i < window_units {
            seen += 1;
            match u32::from(ch) {
                0 => return None,
                0x01..=0x08 | 0x0E..=0x1A | 0x1C..=0x1F | 0x7F => controls += 1,
                _ => {}
            }
        }
    }
    (controls * CONTROL_RATIO <= seen).then_some(encoding)
}

/// The extension entry for the final component of `name`. A name whose only
/// dot is its first character (such as `.profile`) has no extension.
fn extension_entry(name: &str) -> Option<(ExtensionClass, &'static str)> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let (stem, ext) = last.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    let ext = ext.to_ascii_lowercase();
    EXTENSIONS
        .iter()
        .find(|(e, _, _)| *e == ext)
        .map(|(_, class, media)| (*class, *media))
}

/// XML or HTML recognized from the start of UTF-8 text (WHATWG MIME
/// sniffing, as implemented by `infer`), with the media types of
/// [`EXTENSIONS`].
fn text_sniff(text: Option<&[u8]>) -> Option<&'static str> {
    let text = text?;
    let window = &text[..text.len().min(SNIFF_BYTES)];
    if infer::text::is_xml(window) {
        Some("application/xml")
    } else if infer::text::is_html(window) {
        Some("text/html")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] =
        b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";

    #[test]
    fn utf8_text_and_media_types() {
        let d = detect(b"hello\n", Some("notes.txt"));
        assert_eq!(d.class, Class::Text);
        assert_eq!(d.encoding, Some(Encoding::Utf8));
        assert_eq!(d.media_type, "text/plain");
        assert_eq!(d.utf8, Some("hello\n"));
        assert_eq!(
            detect(b"a,b\n", Some("dir/data.CSV")).media_type,
            "text/csv"
        );
        assert_eq!(
            detect(b"a: 1\n", Some("c.yml")).media_type,
            "application/yaml"
        );
        assert_eq!(detect(b"x\n", Some("file.py")).media_type, "text/plain");
        assert_eq!(detect(b"x\n", None).media_type, "text/plain");
        assert_eq!(detect(b"x\n", Some(".profile")).media_type, "text/plain");
        // A text file named .png is still text.
        assert_eq!(detect(b"x\n", Some("a.png")).media_type, "text/plain");
        assert_eq!(
            detect(b"<?xml version=\"1.0\"?><a/>", None).media_type,
            "application/xml"
        );
        assert_eq!(detect(b"  <HTML lang=en>", None).media_type, "text/html");
        // The extension wins over sniffing: a Markdown file may start with a comment.
        assert_eq!(
            detect(b"<!-- note -->\n# Title\n", Some("README.md")).media_type,
            "text/markdown"
        );
        // A BOM is not part of the decoded text.
        assert_eq!(detect(b"\xEF\xBB\xBFhi", None).utf8, Some("hi"));
    }

    #[test]
    fn binary_inputs() {
        let d = detect(PNG, Some("pixel.png"));
        assert_eq!(d.class, Class::Binary);
        assert_eq!(d.encoding, None);
        assert_eq!(d.media_type, "image/png");
        assert_eq!(d.utf8, None);
        // Magic wins over the extension.
        assert_eq!(detect(PNG, Some("pixel.txt")).media_type, "image/png");
        // Latin-1 is not UTF-8.
        let latin1 = detect(b"Caf\xe9\n", Some("menu.txt"));
        assert_eq!(latin1.class, Class::Binary);
        assert_eq!(latin1.media_type, OCTET_STREAM);
        // Without magic, a binary extension entry applies.
        assert_eq!(
            detect(b"\xff\x00\x01", Some("a.PDF")).media_type,
            "application/pdf"
        );
        assert_eq!(
            detect(b"\xff\x00\x01", Some("a.json")).media_type,
            OCTET_STREAM
        );
    }

    #[test]
    fn nul_and_control_characters() {
        let d = detect(b"abc\0def", None);
        assert_eq!(d.class, Class::Binary);
        assert_eq!(d.utf8, Some("abc\0def"), "still valid UTF-8");
        // A NUL after the sniff window does not count.
        let mut late = vec![b'a'; SNIFF_BYTES];
        late.push(0);
        assert_eq!(detect(&late, None).class, Class::Text);
        // ANSI escapes, tabs and form feeds are fine; other controls count.
        assert_eq!(
            detect(b"\x1b[31mred\x1b[0m\t\x0c\n", None).class,
            Class::Text
        );
        assert_eq!(detect(b"\x01\x02abcdefgh", None).class, Class::Binary);
        assert_eq!(detect(b"\x01abcdefghi", None).class, Class::Text);
    }

    #[test]
    fn utf16_and_utf32() {
        let mut le = b"\xFF\xFE".to_vec();
        for unit in "Hi \u{1F600}\n".encode_utf16() {
            le.extend(unit.to_le_bytes());
        }
        let d = detect(&le, Some("a.txt"));
        assert_eq!(
            (d.class, d.encoding, d.media_type, d.utf8),
            (Class::Text, Some(Encoding::Utf16Le), "text/plain", None)
        );
        let mut be = b"\xFE\xFF".to_vec();
        for unit in "Hi\n".encode_utf16() {
            be.extend(unit.to_be_bytes());
        }
        assert_eq!(detect(&be, None).encoding, Some(Encoding::Utf16Be));
        // Odd length, a lone surrogate or UTF-32 are binary.
        assert_eq!(detect(b"\xFF\xFEa\x00b", None).class, Class::Binary);
        assert_eq!(detect(b"\xFF\xFE\x00\xD8a\x00", None).class, Class::Binary);
        assert_eq!(
            detect(b"\xFF\xFE\x00\x00a\x00\x00\x00", None).class,
            Class::Binary
        );
    }

    #[test]
    fn renaming_changes_only_the_media_type() {
        let d = detect(b"a,b\n", Some("x.txt"));
        let renamed = d.renamed(b"a,b\n", Some("x.csv"));
        assert_eq!(renamed.media_type, "text/csv");
        assert_eq!(renamed, detect(b"a,b\n", Some("x.csv")));
    }

    #[test]
    fn empty_input() {
        let d = detect(b"", Some("x.png"));
        assert_eq!((d.class, d.media_type), (Class::Empty, "image/png"));
        assert_eq!(detect(b"", None).media_type, "text/plain");
        assert_eq!(detect(b"", None).utf8, Some(""));
    }

    #[test]
    fn extension_table_is_sorted_lowercase_and_unique() {
        for class in [ExtensionClass::Text, ExtensionClass::Binary] {
            let names: Vec<&str> = EXTENSIONS
                .iter()
                .filter(|(_, c, _)| *c == class)
                .map(|(e, _, _)| *e)
                .collect();
            let mut sorted = names.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(names, sorted);
        }
        for (ext, _, _) in EXTENSIONS {
            assert_eq!(*ext, ext.to_ascii_lowercase());
            assert_eq!(EXTENSIONS.iter().filter(|(e, _, _)| e == ext).count(), 1);
        }
    }

    /// Arbitrary bytes never panic, and the result depends only on the input.
    #[test]
    fn arbitrary_bytes_are_handled() {
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..2000 {
            let len = usize::try_from(next() % 300).unwrap();
            let mut buf: Vec<u8> = (0..len).map(|_| next().to_le_bytes()[0]).collect();
            if round % 3 == 0 && !buf.is_empty() {
                buf[0] = 0xFF;
                if buf.len() > 1 {
                    buf[1] = 0xFE;
                }
            }
            let a = detect(&buf, Some("x.bin"));
            let b = detect(&buf, Some("x.bin"));
            assert_eq!(a, b);
        }
    }
}
