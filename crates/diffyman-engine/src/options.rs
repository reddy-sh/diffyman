//! Options parsing and validation (Diff Document spec §9).
//!
//! An options object with an unknown key, a wrong type (including `null`) or an
//! unknown enum value is invalid and is rejected with [`Error::InvalidOptions`].
//! So is JSON text that gives one key twice in an object: its message starts
//! with `duplicate key`. An `ignore.regex` pattern is only checked when the diff
//! runs: one that does not compile, or is over a limit, gives a document with
//! an `invalid-regex` or `limit-exceeded` error instead (spec §9).
//!
//! Messages quote parts of the input (an unknown key, a duplicate key), so
//! control and bidi-control characters in them are escaped as `\u{..}`: an
//! error printed to a terminal never carries raw escape sequences.

use std::fmt;

use diffyman_model::{
    Adapter, Algorithm, Granularity, IgnoreOptions, MAX_CHILD_BUDGET, MAX_CONTEXT, MAX_TREE_DEPTH,
    Options, OptionsInput, PositionEncoding, TreeOptions, WhitespaceMode,
};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

use crate::Error;

/// Parses and resolves an options JSON text. Blank text means `{}`.
pub fn parse_options(json: &str) -> Result<Options, Error> {
    if json.trim().is_empty() {
        return Ok(Options::default());
    }
    parse_options_value(parse_options_json(json)?)
}

/// Parses options JSON text into a JSON value without resolving it, with the
/// same syntax rules as [`parse_options`]: text that is not JSON, or an
/// object that gives one key twice (at any depth), is
/// [`Error::InvalidOptions`]. For callers that merge other values in before
/// [`parse_options_value`] (the CLI's flags).
pub fn parse_options_json(json: &str) -> Result<Value, Error> {
    let mut de = serde_json::Deserializer::from_str(json);
    let value = Strict::deserialize(&mut de)
        .and_then(|Strict(v)| de.end().map(|()| v))
        .map_err(invalid)?;
    Ok(value)
}

/// Parses and resolves an options JSON value.
pub fn parse_options_value(value: Value) -> Result<Options, Error> {
    check_object(&value, "options")?;
    for key in ["ignore", "tree"] {
        if let Some(inner) = value.get(key) {
            check_object(inner, key)?;
        }
    }
    let input: OptionsInput = serde_json::from_value(value).map_err(invalid)?;
    resolve_options(&input)
}

/// [`Error::InvalidOptions`] with `message`, control characters escaped.
fn invalid(message: impl fmt::Display) -> Error {
    Error::InvalidOptions(escape_controls(&message.to_string()))
}

/// `text` with every control character (C0, DEL, C1) and bidi control
/// written as `\u{..}`, so it is safe to print on a terminal.
pub(crate) fn escape_controls(text: &str) -> String {
    let unsafe_char = |c: char| {
        c.is_control()
            || matches!(
                c,
                '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}'
            )
            || matches!(c, '\u{2066}'..='\u{2069}')
    };
    if !text.chars().any(unsafe_char) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        if unsafe_char(c) {
            out.extend(c.escape_unicode());
        } else {
            out.push(c);
        }
    }
    out
}

/// A JSON value read like [`Value`], except that an object holding the same
/// key twice is an error instead of keeping the last one (RFC 8259 leaves
/// duplicate names to the reader; options must mean one thing).
struct Strict(Value);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Strict, D::Error> {
        deserializer.deserialize_any(StrictVisitor).map(Strict)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        Ok(serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(Strict(item)) = seq.next_element()? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(de::Error::custom(format_args!("duplicate key {key:?}")));
            }
            let Strict(value) = map.next_value()?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

/// Rejects non-objects and `null` members (serde would read `null` as "missing").
fn check_object(value: &Value, what: &str) -> Result<(), Error> {
    let Value::Object(map) = value else {
        return Err(Error::InvalidOptions(format!(
            "{what} must be a JSON object"
        )));
    };
    if let Some((key, _)) = map.iter().find(|(_, v)| v.is_null()) {
        return Err(invalid(format_args!("{what}.{key} must not be null")));
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
    fn duplicate_keys_are_rejected_at_every_depth() {
        for bad in [
            r#"{"context": 1, "context": 1}"#,
            r#"{"context": 1, "algorithm": "myers", "context": 2}"#,
            r#"{"ignore": {"case": true, "case": false}}"#,
            r#"{"ignore": {}, "ignore": {"case": true}}"#,
            r#"{"tree": {"exclude": ["a"], "exclude": ["b"]}}"#,
            r#"{"extra": [{"x": 1, "x": 1}]}"#,
        ] {
            let Err(Error::InvalidOptions(message)) = parse_options(bad) else {
                panic!("accepted {bad}");
            };
            assert!(message.starts_with("duplicate key \""), "{message}");
            let err = parse_options(bad).unwrap_err().to_string();
            assert!(err.starts_with("invalid options: duplicate key"), "{err}");
        }
        // Repeated values in a list are not duplicate keys.
        let o = parse_options(r#"{"ignore": {"regex": ["a", "a"]}}"#).unwrap();
        assert_eq!(o.ignore.regex, ["a", "a"]);
        // The value form is the same as the text form.
        let value = parse_options_json(r#"{"context": 1, "ignore": {"case": true}}"#).unwrap();
        assert_eq!(
            parse_options_value(value).unwrap(),
            parse_options(r#"{"context": 1, "ignore": {"case": true}}"#).unwrap()
        );
        assert!(parse_options_json("{} {}").is_err(), "trailing text");
        assert!(parse_options_json(&"[".repeat(1000)).is_err(), "too deep");
    }

    #[test]
    fn messages_never_carry_raw_control_characters() {
        for bad in [
            "{\"\\u001b[31mred\": 1}",
            "{\"x\u{1b}]0;title\u{7}\": 1}",
            "{\"\\u001b\": 1, \"\\u001b\": 2}",
            "{\"ignore\": {\"\\u202e\\u0000\": true}}",
            "{\"tree\": {\"a\\nb\": null}}",
        ] {
            let err = parse_options(bad).unwrap_err().to_string();
            assert!(err.starts_with("invalid options: "), "{err}");
            assert!(
                !err.chars().any(|c| c.is_control() || c == '\u{202e}'),
                "raw control character in {err:?}"
            );
        }
        assert_eq!(escape_controls("a\u{1b}b\u{202e}c"), "a\\u{1b}b\\u{202e}c");
        assert_eq!(escape_controls("plain é"), "plain é");
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
