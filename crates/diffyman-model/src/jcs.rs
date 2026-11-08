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
use serde::ser::{self, Impossible};
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
///
/// The value is written directly, without an intermediate `serde_json::Value`
/// tree: only the members of the object being written are buffered (to sort
/// them), so peak memory stays close to the size of the output. The text is
/// the same as [`canonicalize`] of `serde_json::to_value(value)`, except that
/// a non-finite float is an error instead of `null`.
pub fn to_canonical_string<T: Serialize + ?Sized>(value: &T) -> Result<String, JcsError> {
    to_canonical_string_without(value, &[])
}

/// Like [`to_canonical_string`], with the top-level object members named in
/// `skip` left out (for the document `id` and the conformance form).
pub fn to_canonical_string_without<T: Serialize + ?Sized>(
    value: &T,
    skip: &[&str],
) -> Result<String, JcsError> {
    let mut out = String::new();
    value
        .serialize(Writer {
            out: &mut out,
            skip,
        })
        .map_err(|e| e.0)?;
    Ok(out)
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

fn write_u64(out: &mut String, u: u64) -> Result<(), JcsError> {
    if u > MAX_SAFE_INTEGER {
        return Err(JcsError::IntegerOutOfRange(u.to_string()));
    }
    // Writing to a String cannot fail.
    let _ = write!(out, "{u}");
    Ok(())
}

fn write_i64(out: &mut String, i: i64) -> Result<(), JcsError> {
    if i.unsigned_abs() > MAX_SAFE_INTEGER {
        return Err(JcsError::IntegerOutOfRange(i.to_string()));
    }
    let _ = write!(out, "{i}");
    Ok(())
}

fn write_number(out: &mut String, n: &Number) -> Result<(), JcsError> {
    if let Some(u) = n.as_u64() {
        write_u64(out, u)?;
    } else if let Some(i) = n.as_i64() {
        write_i64(out, i)?;
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

// ---------------------------------------------------------------------------
// Direct canonical writer
//
// A `serde::Serializer` that writes canonical JSON as it goes. Arrays are
// written in place; an object buffers the canonical text of each member,
// sorts the members and then writes them out. It follows the shapes of
// `serde_json::to_value` (unit and `None` are `null`, a unit variant is its
// name, other variants are a one-member object, bytes are an array of
// numbers, the last of two equal keys wins).
// ---------------------------------------------------------------------------

/// Error of the canonical writer.
#[derive(Debug)]
struct WriteError(JcsError);

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for WriteError {}

impl ser::Error for WriteError {
    fn custom<M: std::fmt::Display>(msg: M) -> Self {
        WriteError(JcsError::Serialize(
            <serde_json::Error as ser::Error>::custom(msg),
        ))
    }
}

impl From<JcsError> for WriteError {
    fn from(e: JcsError) -> Self {
        WriteError(e)
    }
}

type WriteResult<T = ()> = Result<T, WriteError>;

/// Writes one value into `out`.
struct Writer<'a> {
    out: &'a mut String,
    /// Members to leave out if this value is an object.
    skip: &'a [&'a str],
}

impl<'a> Writer<'a> {
    fn object(self, close: &'static str) -> Object<'a> {
        Object {
            out: self.out,
            skip: self.skip,
            members: Vec::new(),
            key: None,
            close,
        }
    }

    /// Opens `{"variant":`, closed by the compound value's `close`.
    fn open_variant(&mut self, variant: &str) {
        self.out.push('{');
        write_string(self.out, variant);
        self.out.push(':');
    }
}

impl<'a> ser::Serializer for Writer<'a> {
    type Ok = ();
    type Error = WriteError;
    type SerializeSeq = Seq<'a>;
    type SerializeTuple = Seq<'a>;
    type SerializeTupleStruct = Seq<'a>;
    type SerializeTupleVariant = Seq<'a>;
    type SerializeMap = Object<'a>;
    type SerializeStruct = Object<'a>;
    type SerializeStructVariant = Object<'a>;

    fn serialize_bool(self, v: bool) -> WriteResult {
        self.out.push_str(if v { "true" } else { "false" });
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> WriteResult {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i16(self, v: i16) -> WriteResult {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i32(self, v: i32) -> WriteResult {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i64(self, v: i64) -> WriteResult {
        Ok(write_i64(self.out, v)?)
    }

    fn serialize_i128(self, v: i128) -> WriteResult {
        if let Ok(u) = u64::try_from(v) {
            Ok(write_u64(self.out, u)?)
        } else if let Ok(i) = i64::try_from(v) {
            Ok(write_i64(self.out, i)?)
        } else {
            Err(JcsError::IntegerOutOfRange(v.to_string()).into())
        }
    }

    fn serialize_u8(self, v: u8) -> WriteResult {
        self.serialize_u64(u64::from(v))
    }

    fn serialize_u16(self, v: u16) -> WriteResult {
        self.serialize_u64(u64::from(v))
    }

    fn serialize_u32(self, v: u32) -> WriteResult {
        self.serialize_u64(u64::from(v))
    }

    fn serialize_u64(self, v: u64) -> WriteResult {
        Ok(write_u64(self.out, v)?)
    }

    fn serialize_u128(self, v: u128) -> WriteResult {
        match u64::try_from(v) {
            Ok(u) => Ok(write_u64(self.out, u)?),
            Err(_) => Err(JcsError::IntegerOutOfRange(v.to_string()).into()),
        }
    }

    fn serialize_f32(self, v: f32) -> WriteResult {
        self.serialize_f64(f64::from(v))
    }

    fn serialize_f64(self, v: f64) -> WriteResult {
        self.out.push_str(&format_f64(v)?);
        Ok(())
    }

    fn serialize_char(self, v: char) -> WriteResult {
        write_string(self.out, v.encode_utf8(&mut [0; 4]));
        Ok(())
    }

    fn serialize_str(self, v: &str) -> WriteResult {
        write_string(self.out, v);
        Ok(())
    }

    fn serialize_bytes(self, v: &[u8]) -> WriteResult {
        self.out.push('[');
        for (i, b) in v.iter().enumerate() {
            if i > 0 {
                self.out.push(',');
            }
            write_u64(self.out, u64::from(*b))?;
        }
        self.out.push(']');
        Ok(())
    }

    fn serialize_none(self) -> WriteResult {
        self.serialize_unit()
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> WriteResult {
        value.serialize(self)
    }

    fn serialize_unit(self) -> WriteResult {
        self.out.push_str("null");
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> WriteResult {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> WriteResult {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> WriteResult {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> WriteResult {
        self.open_variant(variant);
        value.serialize(Writer {
            out: &mut *self.out,
            skip: &[],
        })?;
        self.out.push('}');
        Ok(())
    }

    fn serialize_seq(self, _len: Option<usize>) -> WriteResult<Seq<'a>> {
        self.out.push('[');
        Ok(Seq {
            out: self.out,
            first: true,
            close: "]",
        })
    }

    fn serialize_tuple(self, len: usize) -> WriteResult<Seq<'a>> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> WriteResult<Seq<'a>> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> WriteResult<Seq<'a>> {
        self.open_variant(variant);
        self.out.push('[');
        Ok(Seq {
            out: self.out,
            first: true,
            close: "]}",
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> WriteResult<Object<'a>> {
        Ok(self.object(""))
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> WriteResult<Object<'a>> {
        Ok(self.object(""))
    }

    fn serialize_struct_variant(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> WriteResult<Object<'a>> {
        self.open_variant(variant);
        self.skip = &[];
        Ok(self.object("}"))
    }
}

/// An array (or the array of a tuple variant) being written.
struct Seq<'a> {
    out: &'a mut String,
    first: bool,
    close: &'static str,
}

impl Seq<'_> {
    fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        if !self.first {
            self.out.push(',');
        }
        self.first = false;
        value.serialize(Writer {
            out: &mut *self.out,
            skip: &[],
        })
    }

    fn finish(self) {
        self.out.push_str(self.close);
    }
}

impl ser::SerializeSeq for Seq<'_> {
    type Ok = ();
    type Error = WriteError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        self.element(value)
    }

    fn end(self) -> WriteResult {
        self.finish();
        Ok(())
    }
}

impl ser::SerializeTuple for Seq<'_> {
    type Ok = ();
    type Error = WriteError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        self.element(value)
    }

    fn end(self) -> WriteResult {
        self.finish();
        Ok(())
    }
}

impl ser::SerializeTupleStruct for Seq<'_> {
    type Ok = ();
    type Error = WriteError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        self.element(value)
    }

    fn end(self) -> WriteResult {
        self.finish();
        Ok(())
    }
}

impl ser::SerializeTupleVariant for Seq<'_> {
    type Ok = ();
    type Error = WriteError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        self.element(value)
    }

    fn end(self) -> WriteResult {
        self.finish();
        Ok(())
    }
}

/// An object being written: members are buffered, then sorted and written.
struct Object<'a> {
    out: &'a mut String,
    skip: &'a [&'a str],
    /// (key, canonical text of the value).
    members: Vec<(String, String)>,
    key: Option<String>,
    /// Written after the closing brace (`}` for a struct variant).
    close: &'static str,
}

impl Object<'_> {
    fn member<T: Serialize + ?Sized>(&mut self, key: String, value: &T) -> WriteResult {
        if self.skip.contains(&key.as_str()) {
            return Ok(());
        }
        let mut text = String::new();
        value.serialize(Writer {
            out: &mut text,
            skip: &[],
        })?;
        self.members.push((key, text));
        Ok(())
    }

    fn finish(self) {
        let Object {
            out,
            mut members,
            close,
            ..
        } = self;
        // Stable, so the last of two equal keys stays last.
        members.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
        let mut kept: Vec<(String, String)> = Vec::with_capacity(members.len());
        for member in members {
            if kept.last().is_some_and(|k| k.0 == member.0) {
                kept.pop();
            }
            kept.push(member);
        }
        let size: usize = kept.iter().map(|(k, v)| k.len() + v.len() + 4).sum();
        out.reserve(size + 2 + close.len());
        out.push('{');
        for (i, (key, value)) in kept.into_iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_string(out, &key);
            out.push(':');
            out.push_str(&value);
        }
        out.push('}');
        out.push_str(close);
    }
}

impl ser::SerializeMap for Object<'_> {
    type Ok = ();
    type Error = WriteError;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> WriteResult {
        self.key = Some(key.serialize(KeyWriter)?);
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        let key = self
            .key
            .take()
            .ok_or_else(|| <WriteError as ser::Error>::custom("map value without a key"))?;
        self.member(key, value)
    }

    fn end(self) -> WriteResult {
        self.finish();
        Ok(())
    }
}

impl ser::SerializeStruct for Object<'_> {
    type Ok = ();
    type Error = WriteError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> WriteResult {
        self.member(key.to_owned(), value)
    }

    fn end(self) -> WriteResult {
        self.finish();
        Ok(())
    }
}

impl ser::SerializeStructVariant for Object<'_> {
    type Ok = ();
    type Error = WriteError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> WriteResult {
        self.member(key.to_owned(), value)
    }

    fn end(self) -> WriteResult {
        self.finish();
        Ok(())
    }
}

/// Turns a map key into its string, as `serde_json` does: strings, chars,
/// booleans, integers, unit variants and newtype structs of those.
struct KeyWriter;

fn key_error() -> WriteError {
    <WriteError as ser::Error>::custom("key must be a string")
}

impl ser::Serializer for KeyWriter {
    type Ok = String;
    type Error = WriteError;
    type SerializeSeq = Impossible<String, WriteError>;
    type SerializeTuple = Impossible<String, WriteError>;
    type SerializeTupleStruct = Impossible<String, WriteError>;
    type SerializeTupleVariant = Impossible<String, WriteError>;
    type SerializeMap = Impossible<String, WriteError>;
    type SerializeStruct = Impossible<String, WriteError>;
    type SerializeStructVariant = Impossible<String, WriteError>;

    fn serialize_bool(self, v: bool) -> WriteResult<String> {
        Ok(v.to_string())
    }

    fn serialize_i8(self, v: i8) -> WriteResult<String> {
        Ok(v.to_string())
    }

    fn serialize_i16(self, v: i16) -> WriteResult<String> {
        Ok(v.to_string())
    }

    fn serialize_i32(self, v: i32) -> WriteResult<String> {
        Ok(v.to_string())
    }

    fn serialize_i64(self, v: i64) -> WriteResult<String> {
        Ok(v.to_string())
    }

    fn serialize_i128(self, v: i128) -> WriteResult<String> {
        Ok(v.to_string())
    }

    fn serialize_u8(self, v: u8) -> WriteResult<String> {
        Ok(v.to_string())
    }

    fn serialize_u16(self, v: u16) -> WriteResult<String> {
        Ok(v.to_string())
    }

    fn serialize_u32(self, v: u32) -> WriteResult<String> {
        Ok(v.to_string())
    }

    fn serialize_u64(self, v: u64) -> WriteResult<String> {
        Ok(v.to_string())
    }

    fn serialize_u128(self, v: u128) -> WriteResult<String> {
        Ok(v.to_string())
    }

    fn serialize_f32(self, _v: f32) -> WriteResult<String> {
        Err(key_error())
    }

    fn serialize_f64(self, _v: f64) -> WriteResult<String> {
        Err(key_error())
    }

    fn serialize_char(self, v: char) -> WriteResult<String> {
        Ok(v.to_string())
    }

    fn serialize_str(self, v: &str) -> WriteResult<String> {
        Ok(v.to_owned())
    }

    fn serialize_bytes(self, _v: &[u8]) -> WriteResult<String> {
        Err(key_error())
    }

    fn serialize_none(self) -> WriteResult<String> {
        Err(key_error())
    }

    fn serialize_some<T: Serialize + ?Sized>(self, _value: &T) -> WriteResult<String> {
        Err(key_error())
    }

    fn serialize_unit(self) -> WriteResult<String> {
        Err(key_error())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> WriteResult<String> {
        Err(key_error())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> WriteResult<String> {
        Ok(variant.to_owned())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> WriteResult<String> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> WriteResult<String> {
        Err(key_error())
    }

    fn serialize_seq(self, _len: Option<usize>) -> WriteResult<Self::SerializeSeq> {
        Err(key_error())
    }

    fn serialize_tuple(self, _len: usize) -> WriteResult<Self::SerializeTuple> {
        Err(key_error())
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> WriteResult<Self::SerializeTupleStruct> {
        Err(key_error())
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> WriteResult<Self::SerializeTupleVariant> {
        Err(key_error())
    }

    fn serialize_map(self, _len: Option<usize>) -> WriteResult<Self::SerializeMap> {
        Err(key_error())
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> WriteResult<Self::SerializeStruct> {
        Err(key_error())
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> WriteResult<Self::SerializeStructVariant> {
        Err(key_error())
    }
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

    /// The direct writer gives the same text as canonicalizing `to_value`.
    #[test]
    fn direct_writer_matches_value_path() {
        use std::collections::BTreeMap;

        #[derive(Serialize)]
        enum E {
            Unit,
            New(u32),
            Tuple(i8, String),
            Struct { z: bool, a: Option<u8> },
        }
        #[derive(Serialize)]
        struct Wrap(f64);
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct S {
            zeta: Vec<E>,
            alpha_beta: Option<String>,
            none: Option<u8>,
            map: BTreeMap<String, i64>,
            int_keys: BTreeMap<u32, bool>,
            bytes: Value,
            wrap: Wrap,
            tuple: (u8, char, &'static str),
            nested: Vec<Vec<u64>>,
            #[serde(rename = "\u{20ac}")]
            euro: i32,
            #[serde(rename = "\r")]
            cr: u128,
        }
        let mut map = BTreeMap::new();
        map.insert("b".to_owned(), -3);
        map.insert("\u{1f600}".to_owned(), 9_007_199_254_740_991);
        map.insert("\u{fb33}".to_owned(), -9_007_199_254_740_991);
        let mut int_keys = BTreeMap::new();
        int_keys.insert(10, true);
        int_keys.insert(9, false);
        let s = S {
            zeta: vec![
                E::Unit,
                E::New(7),
                E::Tuple(-1, "x\u{1}\"\\".to_owned()),
                E::Struct { z: true, a: None },
                E::Struct {
                    z: false,
                    a: Some(2),
                },
            ],
            alpha_beta: Some("\u{20ac}$\n\t\u{7f}".to_owned()),
            none: None,
            map,
            int_keys,
            bytes: json!([1, 2, 3]),
            wrap: Wrap(0.7273),
            tuple: (255, 'q', ""),
            nested: vec![vec![], vec![0, 1]],
            euro: -0,
            cr: 42,
        };
        let want = canonicalize(&serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(to_canonical_string(&s).unwrap(), want);
        let mut value = serde_json::to_value(&s).unwrap();
        value.as_object_mut().unwrap().remove("zeta");
        value.as_object_mut().unwrap().remove("map");
        assert_eq!(
            to_canonical_string_without(&s, &["zeta", "map"]).unwrap(),
            canonicalize(&value).unwrap()
        );
    }

    #[test]
    fn direct_writer_rejects_what_jcs_forbids() {
        assert!(to_canonical_string(&9_007_199_254_740_992_u64).is_err());
        assert!(to_canonical_string(&-9_007_199_254_740_992_i64).is_err());
        assert!(to_canonical_string(&u128::MAX).is_err());
        assert!(to_canonical_string(&f64::NAN).is_err());
        assert!(to_canonical_string(&vec![1.0, f64::INFINITY]).is_err());
        let mut floats = std::collections::BTreeMap::new();
        floats.insert(vec![1u8], 1);
        assert!(to_canonical_string(&floats).is_err());
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
