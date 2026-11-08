//! Entry names and paths (spec §19.2 rules 2 and 7).

/// A normalized entry path and whether it is unsafe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Normalized {
    /// `/`-separated, without empty or `.` components; `..` kept as is.
    pub path: String,
    /// Absolute, drive-letter or `..` path (`unsafe-path`).
    pub unsafe_path: bool,
}

/// Normalizes an archive entry name (spec §19.2 rule 7). `None` when the
/// name has no components left, like `./`.
pub(crate) fn normalize(name: &str) -> Option<Normalized> {
    let name = name.replace('\\', "/");
    let absolute = name.starts_with('/');
    let components: Vec<&str> = name
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();
    let first = components.first()?;
    let drive = {
        let b = first.as_bytes();
        b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
    };
    let unsafe_path = absolute || drive || components.contains(&"..");
    let mut path = String::with_capacity(name.len());
    if absolute {
        path.push('/');
    }
    path.push_str(&components.join("/"));
    Some(Normalized { path, unsafe_path })
}

/// Joins folder path components that are already plain names.
#[cfg(feature = "fs")]
pub(crate) fn from_components(components: &[String]) -> Normalized {
    Normalized {
        path: components.join("/"),
        unsafe_path: false,
    }
}

/// Decodes bytes as UTF-8, replacing invalid sequences with U+FFFD and
/// recording that it did.
pub(crate) fn lossy(bytes: &[u8], non_utf8: &mut bool) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        s.to_owned()
    } else {
        *non_utf8 = true;
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// The last path component.
pub(crate) fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(name: &str) -> Option<(String, bool)> {
        normalize(name).map(|p| (p.path, p.unsafe_path))
    }

    #[test]
    fn names_are_normalized_without_resolving() {
        assert_eq!(n("a/b.txt"), Some(("a/b.txt".into(), false)));
        assert_eq!(n("./a//b/./c/"), Some(("a/b/c".into(), false)));
        assert_eq!(n("a\\b\\c.txt"), Some(("a/b/c.txt".into(), false)));
        assert_eq!(n("./"), None);
        assert_eq!(n(""), None);
        assert_eq!(n("/"), None);
        assert_eq!(n("../evil.txt"), Some(("../evil.txt".into(), true)));
        assert_eq!(n("a/../../b"), Some(("a/../../b".into(), true)));
        assert_eq!(n("/etc/passwd"), Some(("/etc/passwd".into(), true)));
        assert_eq!(n("//server/share"), Some(("/server/share".into(), true)));
        assert_eq!(
            n("C:\\Windows\\win.ini"),
            Some(("C:/Windows/win.ini".into(), true))
        );
        assert_eq!(n("c:x"), Some(("c:x".into(), true)));
        assert_eq!(n("..."), Some(("...".into(), false)));
        assert_eq!(n("a..b/.c"), Some(("a..b/.c".into(), false)));
    }

    #[test]
    fn lossy_flags_invalid_utf8() {
        let mut flag = false;
        assert_eq!(lossy(b"ok", &mut flag), "ok");
        assert!(!flag);
        assert_eq!(lossy(b"a\xffb", &mut flag), "a\u{fffd}b");
        assert!(flag);
        assert_eq!(basename("a/b/c.txt"), "c.txt");
        assert_eq!(basename("c.txt"), "c.txt");
    }
}
