//! Diff options from `--options <FILE>` and the direct flags.
//!
//! The file is read as JSON, each flag that is given replaces the matching
//! key (`--ignore-regex` and `--exclude` append to the file's list), and the
//! result goes through the engine's option rules (spec §9), so a flag can
//! never produce options that the JSON form would reject.

use std::path::{Path, PathBuf};

use clap::Args;
use diffyman::model::Options;
use serde_json::{Map, Value};

use crate::input;

/// Option flags shared by `diff`, `dir` and `textconv`.
#[derive(Debug, Clone, Default, Args)]
pub struct IgnoreFlags {
    /// A JSON file with diff options (see `diffyman schema --options`).
    /// Flags given on the command line override its values.
    #[arg(long, value_name = "FILE")]
    pub options: Option<PathBuf>,
    /// Whitespace handling: `none` compares line endings too, `trailing`
    /// ignores trailing whitespace, `change` ignores changes in the amount of
    /// whitespace (`git diff -b`), `all` ignores all whitespace.
    #[arg(long, value_name = "MODE", value_parser = ["none", "trailing", "change", "all"])]
    pub ignore_whitespace: Option<String>,
    /// Compare lines without case (Unicode simple case folding).
    #[arg(short = 'i', long)]
    pub ignore_case: bool,
    /// Mask every match of this regular expression (Rust `regex` syntax)
    /// before lines are compared. Repeatable; appended to the options file's
    /// list.
    #[arg(long, value_name = "REGEX")]
    pub ignore_regex: Vec<String>,
}

/// The remaining option flags of `diff` and `dir`.
#[derive(Debug, Clone, Default, Args)]
pub struct DiffFlags {
    /// Line diff algorithm.
    #[arg(long, value_name = "NAME", value_parser = ["histogram", "myers", "patience"])]
    pub algorithm: Option<String>,
    /// Inline changes inside replaced lines: `line` gives none.
    #[arg(long, value_name = "UNIT", value_parser = ["line", "word", "char", "smart"])]
    pub granularity: Option<String>,
    /// Do not count changes whose lines are all blank.
    #[arg(long)]
    pub ignore_blank_lines: bool,
    /// Lines of context around each change (default 3).
    #[arg(short = 'U', long, value_name = "LINES")]
    pub context: Option<u64>,
    /// Find blocks of lines that moved.
    #[arg(long)]
    pub detect_moves: bool,
    /// Unit of inline offsets in the JSON output.
    #[arg(long, value_name = "UNIT", value_parser = ["utf-8", "utf-16", "codepoint"])]
    pub position_encoding: Option<String>,
    /// How to compare: `auto` detects text, binary and archives.
    #[arg(long, value_name = "NAME", value_parser = ["auto", "text", "binary", "tree"])]
    pub adapter: Option<String>,
    /// Leave out tree entries that match this `.gitignore`-style pattern.
    /// Repeatable; appended to the options file's list.
    #[arg(long, value_name = "PATTERN")]
    pub exclude: Vec<String>,
    /// Folders: honor the `.gitignore` and `.ignore` files inside them.
    #[arg(long)]
    pub ignore_files: bool,
    /// Folders: list what symbolic links point to instead of the links.
    #[arg(long)]
    pub follow_symlinks: bool,
}

/// Reads the options file (if any) as a JSON value. Blank text is `{}`. The
/// file is read up to [`input::MAX_OPTIONS_BYTES`], and parsed with the
/// engine's rules (a key given twice in one object is an error).
fn file_value(path: Option<&Path>) -> Result<Value, String> {
    let Some(path) = path else {
        return Ok(Value::Object(Map::new()));
    };
    let text = String::from_utf8(input::read(path, Some(input::MAX_OPTIONS_BYTES))?)
        .map_err(|_| format!("invalid options: {} is not UTF-8", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    diffyman::parse_options_json(&text).map_err(|e| match e {
        diffyman::Error::InvalidOptions(m) => format!("invalid options: {}: {m}", path.display()),
        other => other.to_string(),
    })
}

/// The object at `key` of `map`, created when missing. `None` when the key
/// holds something else: the value is left alone for the engine to reject.
fn object<'m>(map: &'m mut Map<String, Value>, key: &str) -> Option<&'m mut Map<String, Value>> {
    map.entry(key)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
}

fn set(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), value);
    }
}

fn append(map: &mut Map<String, Value>, key: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    let extra = items.iter().cloned().map(Value::String);
    match map.get_mut(key) {
        Some(Value::Array(list)) => list.extend(extra),
        _ => {
            map.insert(key.to_owned(), Value::Array(extra.collect()));
        }
    }
}

fn flag(on: bool) -> Option<Value> {
    on.then_some(Value::Bool(true))
}

fn string(value: Option<&String>) -> Option<Value> {
    value.cloned().map(Value::String)
}

/// Resolves the options of the file and the flags with the engine's rules.
pub fn resolve(ignore: &IgnoreFlags, diff: &DiffFlags) -> Result<Options, String> {
    let mut value = file_value(ignore.options.as_deref())?;
    if let Value::Object(map) = &mut value {
        set(map, "algorithm", string(diff.algorithm.as_ref()));
        set(map, "granularity", string(diff.granularity.as_ref()));
        set(map, "context", diff.context.map(Value::from));
        set(map, "detectMoves", flag(diff.detect_moves));
        set(
            map,
            "positionEncoding",
            string(diff.position_encoding.as_ref()),
        );
        set(map, "adapter", string(diff.adapter.as_ref()));
        let ignore_given = ignore.ignore_whitespace.is_some()
            || ignore.ignore_case
            || diff.ignore_blank_lines
            || !ignore.ignore_regex.is_empty();
        if ignore_given && let Some(inner) = object(map, "ignore") {
            set(
                inner,
                "whitespace",
                string(ignore.ignore_whitespace.as_ref()),
            );
            set(inner, "case", flag(ignore.ignore_case));
            set(inner, "blankLines", flag(diff.ignore_blank_lines));
            append(inner, "regex", &ignore.ignore_regex);
        }
        let tree_given = diff.ignore_files || diff.follow_symlinks || !diff.exclude.is_empty();
        if tree_given && let Some(inner) = object(map, "tree") {
            set(inner, "ignoreFiles", flag(diff.ignore_files));
            set(inner, "followSymlinks", flag(diff.follow_symlinks));
            append(inner, "exclude", &diff.exclude);
        }
    }
    diffyman::parse_options_value(value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use diffyman::model::{Algorithm, WhitespaceMode};

    #[test]
    fn flags_override_and_append() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("options.json");
        std::fs::write(
            &path,
            r#"{"algorithm": "myers", "context": 1, "ignore": {"regex": ["a"]}}"#,
        )
        .unwrap();
        let ignore = IgnoreFlags {
            options: Some(path),
            ignore_whitespace: Some("all".to_owned()),
            ignore_case: true,
            ignore_regex: vec!["b".to_owned()],
        };
        let diff = DiffFlags {
            context: Some(7),
            detect_moves: true,
            exclude: vec!["*.log".to_owned()],
            ..DiffFlags::default()
        };
        let options = resolve(&ignore, &diff).unwrap();
        assert_eq!(options.algorithm, Algorithm::Myers);
        assert_eq!(options.context, 7);
        assert!(options.detect_moves);
        assert_eq!(options.ignore.whitespace, WhitespaceMode::All);
        assert!(options.ignore.case);
        assert_eq!(options.ignore.regex, ["a", "b"]);
        assert_eq!(options.tree.exclude, ["*.log"]);
    }

    #[test]
    fn engine_rules_still_apply() {
        let diff = DiffFlags {
            context: Some(1 << 31),
            ..DiffFlags::default()
        };
        let err = resolve(&IgnoreFlags::default(), &diff).unwrap_err();
        assert!(err.starts_with("invalid options"), "{err}");
    }
}
