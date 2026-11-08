//! `tree.exclude`: `.gitignore`-style patterns (spec §19.2 rule 8).

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// At most this many patterns.
pub const MAX_PATTERNS: usize = 64;

/// At most this many Unicode scalar values per pattern.
pub const MAX_PATTERN_CHARS: usize = 1024;

/// Why the patterns could not be used. Both can hold at once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExcludeError {
    /// A pattern does not compile (`invalid-glob`).
    pub invalid: bool,
    /// Too many patterns, or a pattern too long (`limit-exceeded`).
    pub limit: bool,
}

/// Compiled `tree.exclude` patterns, matched against entry paths relative
/// to the root of their side.
#[derive(Debug, Clone)]
pub struct Exclude {
    matcher: Gitignore,
}

impl Exclude {
    /// Compiles the patterns, in order (later lines take precedence, as in a
    /// `.gitignore` file). `Ok(None)` when there are none.
    pub fn new(patterns: &[String]) -> Result<Option<Exclude>, ExcludeError> {
        let mut error = ExcludeError {
            invalid: false,
            limit: patterns.len() > MAX_PATTERNS,
        };
        let mut builder = GitignoreBuilder::new("");
        for pattern in patterns {
            if pattern.chars().count() > MAX_PATTERN_CHARS {
                error.limit = true;
                continue;
            }
            if builder.add_line(None, pattern).is_err() {
                error.invalid = true;
            }
        }
        if error.invalid || error.limit {
            return Err(error);
        }
        if patterns.is_empty() {
            return Ok(None);
        }
        let matcher = builder.build().map_err(|_| ExcludeError {
            invalid: true,
            limit: false,
        })?;
        Ok(Some(Exclude { matcher }))
    }

    /// Whether the entry at `path` (normalized, spec §19.2) is left out: the
    /// path or one of its parents matches and is not re-included.
    pub fn excludes(&self, path: &str, is_dir: bool) -> bool {
        // The matcher works on relative paths. An absolute or drive-letter
        // path from an archive is matched as if it were relative.
        let mut relative = path.trim_start_matches('/');
        let b = relative.as_bytes();
        if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            relative = relative[2..].trim_start_matches('/');
        }
        if relative.is_empty() {
            return false;
        }
        self.matcher
            .matched_path_or_any_parents(relative, is_dir)
            .is_ignore()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(patterns: &[&str]) -> Exclude {
        let patterns: Vec<String> = patterns.iter().map(|p| (*p).to_owned()).collect();
        Exclude::new(&patterns).unwrap().unwrap()
    }

    #[test]
    fn gitignore_rules_apply_to_paths_and_parents() {
        let e = ex(&["*.log", "build/", "/top.txt", "!keep.log", "docs/**/draft*"]);
        assert!(e.excludes("a.log", false));
        assert!(e.excludes("x/y/a.log", false));
        assert!(!e.excludes("keep.log", false));
        assert!(e.excludes("build", true));
        assert!(!e.excludes("build", false), "directory-only pattern");
        assert!(e.excludes("build/out.bin", false), "parent matches");
        assert!(e.excludes("top.txt", false));
        assert!(!e.excludes("sub/top.txt", false), "anchored at the root");
        assert!(e.excludes("docs/a/b/draft1.md", false));
        assert!(!e.excludes("docs/final.md", false));
        // Unsafe archive paths are matched as relative ones, never panic.
        assert!(e.excludes("/etc/x.log", false));
        assert!(e.excludes("C:/x.log", false));
        assert!(e.excludes("../x.log", false));
        assert!(!e.excludes("/", true));
    }

    #[test]
    fn errors_and_limits() {
        assert!(Exclude::new(&[]).unwrap().is_none());
        let bad = Exclude::new(&["[z-a]".to_owned()]).unwrap_err();
        assert!(bad.invalid && !bad.limit);
        let many: Vec<String> = (0..=MAX_PATTERNS).map(|i| format!("p{i}")).collect();
        assert!(Exclude::new(&many).unwrap_err().limit);
        assert!(Exclude::new(&many[..MAX_PATTERNS]).is_ok());
        let long = "x".repeat(MAX_PATTERN_CHARS + 1);
        assert!(Exclude::new(&[long]).unwrap_err().limit);
        let both =
            Exclude::new(&["[z-a]".to_owned(), "y".repeat(MAX_PATTERN_CHARS + 1)]).unwrap_err();
        assert!(both.invalid && both.limit);
    }
}
