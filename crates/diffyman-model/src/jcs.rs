//! RFC 8785 JSON Canonicalization Scheme (JCS) over `serde_json::Value`.
//!
//! - Object keys are sorted by their UTF-16 code units.
//! - Strings escape only `"`, `\` and U+0000..U+001F (`\b \t \n \f \r` where they
//!   apply, else `\u00xx` with lowercase hex). Everything else is literal.
//! - Numbers follow ECMAScript `Number.prototype.toString`. Integers must stay
//!   within ±(2^53 − 1) (spec §5).
//! - No insignificant whitespace.

use std::fmt::Write as _;

use serde::Serialize;
use serde_json::{Map, Number, Value};

/// Largest integer JCS (and the Diff Document) allows: 2^53 − 1.
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Errors from canonical serialization.
#[derive(Debug, thiserror::Error)]
pub enum JcsError {
    /// An integer outside ±(2^53 − 1).
    #[error("integer {0} is outside the range JCS allows (±(2^53 - 1))")]
    IntegerOutOfRange(String),
    /// NaN or an infinity.
    #[error("non-finite numbers are not allowed in JCS")]
    NonFinite,
    /// The value could not be converted to JSON.
    #[error("serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Serializes any `Serialize` value to canonical JSON text.
pub fn to_canonical_string<T: Serialize + ?Sized>(value: &T) -> Result<String, JcsError> {
    let value = serde_json::to_value(value)?;
    canonicalize(&value)
}

/// Serializes a JSON value to canonical JSON text.
pub fn canonicalize(value: &Value) -> Result<String, JcsError> {
    let mut out = String::new();
    write_value(&mut out, value)?;
    Ok(out)
}

fn write_value(out: &mut String, value: &Value) -> Result<(), JcsError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => write_number(out, n)?,
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item)?;
            }
            out.push(']');
        }
        Value::Object(map) => write_object(out, map)?,
    }
    Ok(())
}

fn write_object(out: &mut String, map: &Map<String, Value>) -> Result<(), JcsError> {
    let mut entries: Vec<(&String, &Value)> = map.iter().collect();
    entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
    out.push('{');
    for (i, (key, value)) in entries.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_string(out, key);
        out.push(':');
        write_value(out, value)?;
    }
    out.push('}');
    Ok(())
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if u32::from(c) < 0x20 => {
                // Writing to a String cannot fail.
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_number(out: &mut String, n: &Number) -> Result<(), JcsError> {
    if let Some(u) = n.as_u64() {
        if u > MAX_SAFE_INTEGER {
            return Err(JcsError::IntegerOutOfRange(u.to_string()));
        }
        let _ = write!(out, "{u}");
    } else if let Some(i) = n.as_i64() {
        if i.unsigned_abs() > MAX_SAFE_INTEGER {
            return Err(JcsError::IntegerOutOfRange(i.to_string()));
        }
        let _ = write!(out, "{i}");
    } else if let Some(f) = n.as_f64() {
        out.push_str(&format_f64(f)?);
    } else {
        return Err(JcsError::NonFinite);
    }
    Ok(())
}

/// Formats a double like ECMAScript `Number.prototype.toString` (ECMA-262 §6.1.6.1.20).
pub fn format_f64(x: f64) -> Result<String, JcsError> {
    if !x.is_finite() {
        return Err(JcsError::NonFinite);
    }
    if x == 0.0 {
        // ECMAScript prints both +0 and -0 as "0".
        return Ok("0".to_owned());
    }
    // `ryu-js` implements the ECMAScript algorithm, including the rule that a
    // tie between two shortest digit strings goes to the even one (Rust's own
    // shortest formatting does not follow that rule).
    let mut buffer = ryu_js::Buffer::new();
    Ok(buffer.format_finite(x).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bits(b: u64) -> String {
        format_f64(f64::from_bits(b)).unwrap()
    }

    /// RFC 8785 Appendix B number samples.
    #[test]
    fn rfc8785_appendix_b_numbers() {
        let cases: &[(u64, &str)] = &[
            (0x0000_0000_0000_0000, "0"),
            (0x8000_0000_0000_0000, "0"),
            (0x0000_0000_0000_0001, "5e-324"),
            (0x8000_0000_0000_0001, "-5e-324"),
            (0x7fef_ffff_ffff_ffff, "1.7976931348623157e+308"),
            (0xffef_ffff_ffff_ffff, "-1.7976931348623157e+308"),
            (0x4340_0000_0000_0000, "9007199254740992"),
            (0xc340_0000_0000_0000, "-9007199254740992"),
            (0x4430_0000_0000_0000, "295147905179352830000"),
            (0x44b5_2d02_c7e1_4af5, "9.999999999999997e+22"),
            (0x44b5_2d02_c7e1_4af6, "1e+23"),
            (0x44b5_2d02_c7e1_4af7, "1.0000000000000001e+23"),
            (0x444b_1ae4_d6e2_ef4e, "999999999999999700000"),
            (0x444b_1ae4_d6e2_ef4f, "999999999999999900000"),
            (0x444b_1ae4_d6e2_ef50, "1e+21"),
            (0x3eb0_c6f7_a0b5_ed8c, "9.999999999999997e-7"),
            (0x3eb0_c6f7_a0b5_ed8d, "0.000001"),
            (0x41b3_de43_5555_5553, "333333333.3333332"),
            (0x41b3_de43_5555_5554, "333333333.33333325"),
            (0x41b3_de43_5555_5555, "333333333.3333333"),
            (0x41b3_de43_5555_5556, "333333333.3333334"),
            (0x41b3_de43_5555_5557, "333333333.33333343"),
            (0xbecb_f647_612f_3696, "-0.0000033333333333333333"),
            (0x4314_3ff3_c1cb_0959, "1424953923781206.2"),
        ];
        for (b, want) in cases {
            assert_eq!(bits(*b), *want, "bits {b:#018x}");
        }
    }

    #[test]
    fn non_finite_is_rejected() {
        assert!(format_f64(f64::NAN).is_err());
        assert!(format_f64(f64::INFINITY).is_err());
    }

    /// RFC 8785 §3.2.2 example (numbers, string escapes, literals).
    #[test]
    fn rfc8785_serialization_example() {
        let input = r#"{
          "numbers": [333333333.33333329, 1E30, 4.50, 2e-3, 0.000000000000000000000000001],
          "string": "\u20ac$\u000F\u000aA'\u0042\u0022\u005c\\\"\/",
          "literals": [null, true, false]
        }"#;
        let value: Value = serde_json::from_str(input).unwrap();
        let want = concat!(
            r#"{"literals":[null,true,false],"numbers":[333333333.3333333,1e+30,4.5,0.002,1e-27],"#,
            r#""string":"€$\u000f\nA'B\"\\\\\"/"}"#
        );
        assert_eq!(canonicalize(&value).unwrap(), want);
    }

    /// RFC 8785 §3.2.3 example: property sorting by UTF-16 code units.
    #[test]
    fn rfc8785_sorting_example() {
        let input = r#"{
          "\u20ac": "Euro Sign",
          "\r": "Carriage Return",
          "\ufb33": "Hebrew Letter Dalet With Dagesh",
          "1": "One",
          "\ud83d\ude00": "Emoji: Grinning Face",
          "\u0080": "Control",
          "\u00f6": "Latin Small Letter O With Diaeresis"
        }"#;
        let value: Value = serde_json::from_str(input).unwrap();
        let want = concat!(
            "{\"\\r\":\"Carriage Return\",\"1\":\"One\",\"\u{80}\":\"Control\",",
            "\"\u{f6}\":\"Latin Small Letter O With Diaeresis\",\"\u{20ac}\":\"Euro Sign\",",
            "\"\u{1f600}\":\"Emoji: Grinning Face\",",
            "\"\u{fb33}\":\"Hebrew Letter Dalet With Dagesh\"}"
        );
        assert_eq!(canonicalize(&value).unwrap(), want);
    }

    #[test]
    fn nested_values_and_integers() {
        let value = json!({"b": [1, -2, {"z": null, "a": true}], "a": "x\u{1}y", "c": 0.7273});
        assert_eq!(
            canonicalize(&value).unwrap(),
            r#"{"a":"x\u0001y","b":[1,-2,{"a":true,"z":null}],"c":0.7273}"#
        );
    }

    #[test]
    fn integer_range_is_enforced() {
        assert!(canonicalize(&json!(9_007_199_254_740_991_u64)).is_ok());
        assert!(canonicalize(&json!(9_007_199_254_740_992_u64)).is_err());
        assert!(canonicalize(&json!(-9_007_199_254_740_992_i64)).is_err());
    }

    #[test]
    fn four_decimal_similarities() {
        for n in 0..=10_000_u32 {
            let s = format_f64(f64::from(n) / 10_000.0).unwrap();
            let back: f64 = s.parse().unwrap();
            assert_eq!(back, f64::from(n) / 10_000.0, "{n} -> {s}");
            let decimals = s.split_once('.').map_or(0, |(_, d)| d.len());
            assert!(decimals <= 4, "{n} -> {s}");
            assert!(!s.contains('e'), "{n} -> {s}");
        }
    }
}
