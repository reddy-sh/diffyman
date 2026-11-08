//! Options parsing and validation (Diff Document spec §9).
//!
//! An options object with an unknown key, a wrong type (including `null`) or an
//! unknown enum value is invalid and is rejected with [`Error::InvalidOptions`].
//! An `ignore.regex` pattern is only checked when the diff runs: one that does
//! not compile, or is over a limit, gives a document with an `invalid-regex` or
//! `limit-exceeded` error instead (spec §9).

use diffyman_model::{
    Adapter, Algorithm, Granularity, IgnoreOptions, MAX_CHILD_BUDGET, MAX_CONTEXT, MAX_TREE_DEPTH,
    Options, OptionsInput, PositionEncoding, TreeOptions, WhitespaceMode,
};
use serde_json::Value;

use crate::Error;

/// Parses and resolves an options JSON text. Blank text means `{}`.
pub fn parse_options(json: &str) -> Result<Options, Error> {
    if json.trim().is_empty() {
        return Ok(Options::default());
    }
    let value: Value =
        serde_json::from_str(json).map_err(|e| Error::InvalidOptions(e.to_string()))?;
    parse_options_value(value)
}

/// Parses and resolves an options JSON value.
pub fn parse_options_value(value: Value) -> Result<Options, Error> {
    check_object(&value, "options")?;
    for key in ["ignore", "tree"] {
        if let Some(inner) = value.get(key) {
            check_object(inner, key)?;
        }
    }
    let input: OptionsInput =
        serde_json::from_value(value).map_err(|e| Error::InvalidOptions(e.to_string()))?;
    resolve_options(&input)
}

/// Rejects non-objects and `null` members (serde would read `null` as "missing").
fn check_object(value: &Value, what: &str) -> Result<(), Error> {
    let Value::Object(map) = value else {
        return Err(Error::InvalidOptions(format!(
            "{what} must be a JSON object"
        )));
    };
    if let Some((key, _)) = map.iter().find(|(_, v)| v.is_null()) {
        return Err(Error::InvalidOptions(format!(
            "{what}.{key} must not be null"
        )));
    }
    Ok(())
}

/// Fills defaults and validates values.
pub fn resolve_options(input: &OptionsInput) -> Result<Options, Error> {
    let defaults = Options::default();
    let ignore = input.ignore.clone().unwrap_or_default();
    let options = Options {
        algorithm: input.algorithm.unwrap_or(defaults.algorithm),
        granularity: input.granularity.unwrap_or(defaults.granularity),
        ignore: IgnoreOptions {
            whitespace: ignore.whitespace.unwrap_or(defaults.ignore.whitespace),
            case: ignore.case.unwrap_or(defaults.ignore.case),
            blank_lines: ignore.blank_lines.unwrap_or(defaults.ignore.blank_lines),
            regex: ignore.regex.unwrap_or_default(),
        },
        context: match input.context {
            None => defaults.context,
            Some(c) => u32::try_from(c)
                .ok()
                .filter(|c| *c <= MAX_CONTEXT)
                .ok_or_else(|| {
                    Error::InvalidOptions(format!("context must be 0 to {MAX_CONTEXT}"))
                })?,
        },
        position_encoding: input
            .position_encoding
            .unwrap_or(defaults.position_encoding),
        detect_moves: input.detect_moves.unwrap_or(defaults.detect_moves),
        adapter: input.adapter.unwrap_or(defaults.adapter),
        tree: resolve_tree(input.tree.clone().unwrap_or_default(), &defaults.tree)?,
    };
    validate_options(&options)?;
    Ok(options)
}

/// Fills the `tree` defaults and checks the integer ranges.
fn resolve_tree(
    input: diffyman_model::TreeOptionsInput,
    defaults: &TreeOptions,
) -> Result<TreeOptions, Error> {
    let bounded = |value: Option<u64>, default: u32, max: u32, what: &str| match value {
        None => Ok(default),
        Some(v) => u32::try_from(v)
            .ok()
            .filter(|v| *v <= max)
            .ok_or_else(|| Error::InvalidOptions(format!("{what} must be 0 to {max}"))),
    };
    Ok(TreeOptions {
        follow_symlinks: input.follow_symlinks.unwrap_or(defaults.follow_symlinks),
        ignore_files: input.ignore_files.unwrap_or(defaults.ignore_files),
        exclude: input.exclude.unwrap_or_default(),
        detect_moves: input.detect_moves.unwrap_or(defaults.detect_moves),
        max_depth: bounded(
            input.max_depth,
            defaults.max_depth,
            MAX_TREE_DEPTH,
            "tree.maxDepth",
        )?,
        child_budget: bounded(
            input.child_budget,
            defaults.child_budget,
            MAX_CHILD_BUDGET,
            "tree.childBudget",
        )?,
    })
}

/// Checks resolved options with the same rules as the JSON path: an `Unknown`
/// enum value or a `context` above 2^31 − 1 is [`Error::InvalidOptions`].
///
/// The typed API ([`crate::diff_document`]) calls this, so a caller-built
/// [`Options`] can never produce a document that echoes `"unknown"`.
pub fn validate_options(options: &Options) -> Result<(), Error> {
    let unknown = |what: &str| Err(Error::InvalidOptions(format!("unknown value for {what}")));
    if options.algorithm == Algorithm::Unknown {
        return unknown("algorithm");
    }
    if options.granularity == Granularity::Unknown {
        return unknown("granularity");
    }
    if options.ignore.whitespace == WhitespaceMode::Unknown {
        return unknown("ignore.whitespace");
    }
    if options.position_encoding == PositionEncoding::Unknown {
        return unknown("positionEncoding");
    }
    if options.adapter == Adapter::Unknown {
        return unknown("adapter");
    }
    if options.context > MAX_CONTEXT {
        return Err(Error::InvalidOptions(format!(
            "context must be 0 to {MAX_CONTEXT}"
        )));
    }
    if options.tree.max_depth > MAX_TREE_DEPTH {
        return Err(Error::InvalidOptions(format!(
            "tree.maxDepth must be 0 to {MAX_TREE_DEPTH}"
        )));
    }
    if options.tree.child_budget > MAX_CHILD_BUDGET {
        return Err(Error::InvalidOptions(format!(
            "tree.childBudget must be 0 to {MAX_CHILD_BUDGET}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        assert_eq!(parse_options("{}").unwrap(), Options::default());
        assert_eq!(parse_options("").unwrap(), Options::default());
        let o = parse_options(r#"{"context": 1, "ignore": {"case": true}}"#).unwrap();
        assert_eq!(o.context, 1);
        assert!(o.ignore.case);
        assert_eq!(o.ignore.whitespace, WhitespaceMode::None);
        assert_eq!(o.adapter, Adapter::Auto);
        assert_eq!(o.tree, TreeOptions::default());
        let t = parse_options(
            r#"{"tree": {"exclude": ["b", "a"], "maxDepth": 16, "childBudget": 0, "detectMoves": false}}"#,
        )
        .unwrap()
        .tree;
        assert_eq!(t.exclude, ["b", "a"]);
        assert_eq!((t.max_depth, t.child_budget), (16, 0));
        assert!(!t.detect_moves && !t.follow_symlinks && !t.ignore_files);
        for (text, adapter) in [
            ("auto", Adapter::Auto),
            ("text", Adapter::Text),
            ("binary", Adapter::Binary),
            ("tree", Adapter::Tree),
        ] {
            let json = format!(r#"{{"adapter": "{text}"}}"#);
            assert_eq!(parse_options(&json).unwrap().adapter, adapter);
        }
    }

    #[test]
    fn invalid_options_are_rejected() {
        for bad in [
            r#"{"contxt": 1}"#,
            r#"{"ignore": {"cases": true}}"#,
            r#"{"algorithm": "diff3"}"#,
            r#"{"granularity": "sentence"}"#,
            r#"{"positionEncoding": "utf-32"}"#,
            r#"{"ignore": {"whitespace": "some"}}"#,
            r#"{"adapter": "hex"}"#,
            r#"{"adapter": null}"#,
            r#"{"context": -1}"#,
            r#"{"context": 2147483648}"#,
            r#"{"context": 1.5}"#,
            r#"{"context": "3"}"#,
            r#"{"algorithm": null}"#,
            r#"{"ignore": null}"#,
            r#"{"ignore": {"case": null}}"#,
            r#"{"ignore": []}"#,
            r#"{"adapter": "folder"}"#,
            r#"{"tree": null}"#,
            r#"{"tree": []}"#,
            r#"{"tree": {"follow": true}}"#,
            r#"{"tree": {"followSymlinks": null}}"#,
            r#"{"tree": {"exclude": "*.log"}}"#,
            r#"{"tree": {"exclude": [1]}}"#,
            r#"{"tree": {"maxDepth": 17}}"#,
            r#"{"tree": {"maxDepth": -1}}"#,
            r#"{"tree": {"childBudget": 2147483648}}"#,
            r#"{"tree": {"detectMoves": "yes"}}"#,
            "[]",
            "3",
            "{",
        ] {
            assert!(
                matches!(parse_options(bad), Err(Error::InvalidOptions(_))),
                "accepted {bad}"
            );
        }
        assert_eq!(
            parse_options(r#"{"context": 2147483647}"#).unwrap().context,
            MAX_CONTEXT
        );
    }

    #[test]
    fn typed_options_are_validated_like_json() {
        assert!(validate_options(&Options::default()).is_ok());
        let max = Options {
            context: MAX_CONTEXT,
            ..Options::default()
        };
        assert!(validate_options(&max).is_ok());
        let bad = [
            Options {
                algorithm: Algorithm::Unknown,
                ..Options::default()
            },
            Options {
                granularity: Granularity::Unknown,
                ..Options::default()
            },
            Options {
                ignore: IgnoreOptions {
                    whitespace: WhitespaceMode::Unknown,
                    ..IgnoreOptions::default()
                },
                ..Options::default()
            },
            Options {
                position_encoding: PositionEncoding::Unknown,
                ..Options::default()
            },
            Options {
                adapter: Adapter::Unknown,
                ..Options::default()
            },
            Options {
                context: MAX_CONTEXT + 1,
                ..Options::default()
            },
            Options {
                context: u32::MAX,
                ..Options::default()
            },
            Options {
                tree: TreeOptions {
                    max_depth: MAX_TREE_DEPTH + 1,
                    ..TreeOptions::default()
                },
                ..Options::default()
            },
            Options {
                tree: TreeOptions {
                    child_budget: MAX_CHILD_BUDGET + 1,
                    ..TreeOptions::default()
                },
                ..Options::default()
            },
            Options::unset(),
        ];
        for options in bad {
            assert!(
                matches!(validate_options(&options), Err(Error::InvalidOptions(_))),
                "accepted {options:?}"
            );
        }
    }

    #[test]
    fn regex_patterns_are_kept_in_order_and_checked_later() {
        let o =
            parse_options(r#"{"ignore": {"regex": ["b", "(", "a"], "blankLines": true}}"#).unwrap();
        assert_eq!(o.ignore.regex, ["b", "(", "a"]);
        assert!(o.ignore.blank_lines);
        for bad in [
            r#"{"ignore": {"regex": "a"}}"#,
            r#"{"ignore": {"regex": [1]}}"#,
            r#"{"ignore": {"regex": [null]}}"#,
            r#"{"ignore": {"blankLines": "yes"}}"#,
        ] {
            assert!(
                matches!(parse_options(bad), Err(Error::InvalidOptions(_))),
                "accepted {bad}"
            );
        }
    }
}
