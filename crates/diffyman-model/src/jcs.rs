//! RFC 8785 JSON Canonicalization Scheme (JCS).
//!
//! - Object keys are sorted by their UTF-16 code units.
//! - Strings escape only `"`, `\` and U+0000..U+001F (`\b \t \n \f \r` where they
//!   apply, else `\u00xx` with lowercase hex). Everything else is literal.
//! - Numbers follow ECMAScript `Number.prototype.toString`. Integers must stay
//!   within ±(2^53 − 1), and so must every number written without an exponent:
//!   a float of magnitude 2^53 up to 10^21, which ECMAScript prints as a plain
//!   integer literal, is rejected like an out-of-range integer (spec §5). So
//!   canonical text always reads back as the same canonical text.
//! - No insignificant whitespace.
//!
//! Any `Serialize` value can be written to a `String` ([`to_canonical_string`]),
//! to an [`io::Write`] ([`to_canonical_writer`]) or into a SHA-256
//! ([`canonical_sha256_hex`]) without building the text or a
//! `serde_json::Value` tree first.

use std::borrow::Cow;
use std::io;

use serde::Serialize;
use serde::ser::{self, Impossible};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

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
    /// The output could not be written.
    #[error("write failed: {0}")]
    Io(#[from] io::Error),
}

/// Serializes any `Serialize` value to canonical JSON text.
///
/// The text is the same as [`canonicalize`] of `serde_json::to_value(value)`,
/// except that a non-finite float is an error instead of `null`.
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
    write_value(&mut out, value, skip).map_err(|e| e.0)?;
    Ok(out)
}

/// Writes the canonical JSON text of `value` to `writer`, as it is produced.
///
/// Memory use does not grow with the size of the text: nothing is buffered
/// beyond the key list of the objects being written. Pass a buffered writer
/// (`io::BufWriter`); the text is written in many small pieces.
pub fn to_canonical_writer<T: Serialize + ?Sized, W: io::Write>(
    value: &T,
    writer: W,
) -> Result<(), JcsError> {
    to_canonical_writer_without(value, &[], writer)
}

/// Like [`to_canonical_writer`], with top-level members named in `skip` left out.
pub fn to_canonical_writer_without<T: Serialize + ?Sized, W: io::Write>(
    value: &T,
    skip: &[&str],
    writer: W,
) -> Result<(), JcsError> {
    let mut sink = IoSink(writer);
    write_value(&mut sink, value, skip).map_err(|e| e.0)
}

/// Lowercase hex SHA-256 of the canonical JSON text of `value` without the
/// top-level members named in `skip`, computed without holding the text.
pub fn canonical_sha256_hex<T: Serialize + ?Sized>(
    value: &T,
    skip: &[&str],
) -> Result<String, JcsError> {
    let mut sink = HashSink {
        hasher: Sha256::new(),
        buf: Vec::with_capacity(HashSink::CAPACITY),
    };
    write_value(&mut sink, value, skip).map_err(|e| e.0)?;
    Ok(sink.finish())
}

/// Serializes a JSON value to canonical JSON text.
pub fn canonicalize(value: &Value) -> Result<String, JcsError> {
    let mut out = String::new();
    write_json_value(&mut out, value)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Output sinks
// ---------------------------------------------------------------------------

/// Where canonical text goes.
trait Sink {
    fn put(&mut self, s: &str) -> Result<(), JcsError>;
}

impl Sink for String {
    #[inline]
    fn put(&mut self, s: &str) -> Result<(), JcsError> {
        self.push_str(s);
        Ok(())
    }
}

struct IoSink<W>(W);

impl<W: io::Write> Sink for IoSink<W> {
    #[inline]
    fn put(&mut self, s: &str) -> Result<(), JcsError> {
        Ok(self.0.write_all(s.as_bytes())?)
    }
}

/// Hashes the text in blocks, so tiny pieces do not each reach the hasher.
struct HashSink {
    hasher: Sha256,
    buf: Vec<u8>,
}

impl HashSink {
    const CAPACITY: usize = 64 * 1024;

    fn finish(mut self) -> String {
        self.hasher.update(&self.buf);
        let digest = self.hasher.finalize();
        let mut out = String::with_capacity(64);
        for byte in &digest {
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        out
    }
}

impl Sink for HashSink {
    #[inline]
    fn put(&mut self, s: &str) -> Result<(), JcsError> {
        if self.buf.len() + s.len() > Self::CAPACITY {
            self.hasher.update(&self.buf);
            self.buf.clear();
            if s.len() > Self::CAPACITY {
                self.hasher.update(s.as_bytes());
                return Ok(());
            }
        }
        self.buf.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

// ---------------------------------------------------------------------------
// Scalars
// ---------------------------------------------------------------------------

fn write_json_value<S: Sink>(out: &mut S, value: &Value) -> Result<(), JcsError> {
    match value {
        Value::Null => out.put("null")?,
        Value::Bool(true) => out.put("true")?,
        Value::Bool(false) => out.put("false")?,
        Value::Number(n) => write_number(out, n)?,
        Value::String(s) => write_string(out, s)?,
        Value::Array(items) => {
            out.put("[")?;
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.put(",")?;
                }
                write_json_value(out, item)?;
            }
            out.put("]")?;
        }
        Value::Object(map) => write_json_object(out, map)?,
    }
    Ok(())
}

fn write_json_object<S: Sink>(out: &mut S, map: &Map<String, Value>) -> Result<(), JcsError> {
    let mut entries: Vec<(&String, &Value)> = map.iter().collect();
    entries.sort_by(|a, b| utf16_cmp(a.0, b.0));
    out.put("{")?;
    for (i, (key, value)) in entries.into_iter().enumerate() {
        if i > 0 {
            out.put(",")?;
        }
        write_string(out, key)?;
        out.put(":")?;
        write_json_value(out, value)?;
    }
    out.put("}")
}

/// Compares two keys by their UTF-16 code units (RFC 8785 §3.2.3).
fn utf16_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    if a.is_ascii() && b.is_ascii() {
        a.cmp(b)
    } else {
        a.encode_utf16().cmp(b.encode_utf16())
    }
}

fn write_string<S: Sink>(out: &mut S, s: &str) -> Result<(), JcsError> {
    out.put("\"")?;
    let bytes = s.as_bytes();
    let mut run = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let escape = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            0x08 => "\\b",
            b'\t' => "\\t",
            b'\n' => "\\n",
            0x0c => "\\f",
            b'\r' => "\\r",
            0x00..=0x1f => "",
            _ => continue,
        };
        // Escaped bytes are ASCII, so `run..i` lies on character boundaries.
        out.put(&s[run..i])?;
        if escape.is_empty() {
            let hex = [
                b'\\',
                b'u',
                b'0',
                b'0',
                HEX[usize::from(b >> 4)],
                HEX[usize::from(b & 0x0f)],
            ];
            out.put(std::str::from_utf8(&hex).unwrap_or_default())?;
        } else {
            out.put(escape)?;
        }
        run = i + 1;
    }
    out.put(&s[run..])?;
    out.put("\"")
}

/// Decimal digits of `u` without allocating.
fn write_decimal<S: Sink>(out: &mut S, negative: bool, mut u: u64) -> Result<(), JcsError> {
    let mut buf = [0u8; 21];
    let mut i = buf.len();
    loop {
        i -= 1;
        // A single digit always fits.
        #[allow(clippy::cast_possible_truncation)]
        let digit = (u % 10) as u8;
        buf[i] = b'0' + digit;
        u /= 10;
        if u == 0 {
            break;
        }
    }
    if negative {
        i -= 1;
        buf[i] = b'-';
    }
    out.put(std::str::from_utf8(&buf[i..]).unwrap_or_default())
}

fn write_u64<S: Sink>(out: &mut S, u: u64) -> Result<(), JcsError> {
    if u > MAX_SAFE_INTEGER {
        return Err(JcsError::IntegerOutOfRange(u.to_string()));
    }
    write_decimal(out, false, u)
}

fn write_i64<S: Sink>(out: &mut S, i: i64) -> Result<(), JcsError> {
    if i.unsigned_abs() > MAX_SAFE_INTEGER {
        return Err(JcsError::IntegerOutOfRange(i.to_string()));
    }
    write_decimal(out, i < 0, i.unsigned_abs())
}

fn write_number<S: Sink>(out: &mut S, n: &Number) -> Result<(), JcsError> {
    if let Some(u) = n.as_u64() {
        write_u64(out, u)
    } else if let Some(i) = n.as_i64() {
        write_i64(out, i)
    } else if let Some(f) = n.as_f64() {
        out.put(&format_number(f)?)
    } else {
        Err(JcsError::NonFinite)
    }
}

/// Smallest magnitude ECMAScript prints with an exponent (`1e+21`).
const EXPONENT_FROM: f64 = 1e21;

/// Formats a double for canonical output: [`format_f64`], except that a value
/// of magnitude from 2^53 up to (not including) 10^21 is an error. ECMAScript
/// prints those as integer literals (`1e19` as `10000000000000000000`), which
/// read back as integers outside ±(2^53 − 1), so the text would not be
/// canonical a second time (spec §5).
pub fn format_number(x: f64) -> Result<String, JcsError> {
    let magnitude = x.abs();
    #[allow(clippy::cast_precision_loss)] // 2^53 is exact as a double
    let first_unsafe = (MAX_SAFE_INTEGER + 1) as f64;
    if (first_unsafe..EXPONENT_FROM).contains(&magnitude) {
        return Err(JcsError::IntegerOutOfRange(format_f64(x)?));
    }
    format_f64(x)
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
// Canonical text is written as it goes, following the shapes of
// `serde_json::to_value` (unit and `None` are `null`, a unit variant is its
// name, other variants are a one-member object, bytes are an array of
// numbers, the last of two equal keys wins).
//
// Objects are the hard part: their members must come out sorted, but serde
// hands them over in declaration order. Instead of buffering each member's
// text, `write_value` serializes an object value several times:
//
// 1. A `Probe` pass records the keys and skips every value.
// 2. The keys are sorted (UTF-16 order, last of equal keys kept).
// 3. For each key in that order, a `Select` pass serializes the value again
//    and writes only that member, recursively through `write_value`.
//
// `Serialize` takes `&self`, so every pass sees the same value. Each member is
// written exactly once; the extra passes only walk the field list. Arrays are
// written element by element. Peak memory is the key lists of the objects
// being written, whatever the size of the text.
//
// The one shape that cannot be re-serialized from its parent is a struct
// variant (its fields arrive inside the variant's own `serialize` call). Its
// members are buffered as before; the document types have none.
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

fn inconsistent() -> WriteError {
    <WriteError as ser::Error>::custom("value serialized differently on a second pass")
}

/// Writes one value: objects through the probe and select passes, anything
/// else directly. `skip` names members to leave out if it is an object.
fn write_value<T: Serialize + ?Sized, S: Sink>(
    out: &mut S,
    value: &T,
    skip: &[&str],
) -> WriteResult {
    let keys = match value.serialize(Probe) {
        Ok(Shape::Object(keys)) => keys,
        Ok(Shape::Other) | Err(ProbeError::Other) => return value.serialize(Writer { out }),
        Err(ProbeError::Real(e)) => return Err(e),
    };
    // (position in serialization order, key), sorted; the sort is stable, so
    // of two equal keys the later one stays later and wins.
    let mut order: Vec<(usize, &str)> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| (i, k.as_ref()))
        .filter(|(_, k)| !skip.contains(k))
        .collect();
    order.sort_by(|a, b| utf16_cmp(a.1, b.1));
    out.put("{")?;
    let mut first = true;
    for (n, &(index, key)) in order.iter().enumerate() {
        if order.get(n + 1).is_some_and(|next| next.1 == key) {
            continue;
        }
        if !first {
            out.put(",")?;
        }
        first = false;
        write_string(out, key)?;
        out.put(":")?;
        let found = value.serialize(Select {
            out: &mut *out,
            target: index,
        })?;
        if !found {
            return Err(inconsistent());
        }
    }
    out.put("}")?;
    Ok(())
}

/// What a probe pass found.
enum Shape {
    /// An object (struct or map) with these keys, in serialization order.
    Object(Vec<Cow<'static, str>>),
    /// Anything else.
    Other,
}

#[derive(Debug)]
enum ProbeError {
    /// Not an object: stops the probe early (for example on a long array).
    Other,
    /// A real error, such as a map key that is not a string.
    Real(WriteError),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::Other => f.write_str("not an object"),
            ProbeError::Real(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ProbeError {}

impl ser::Error for ProbeError {
    fn custom<M: std::fmt::Display>(msg: M) -> Self {
        ProbeError::Real(<WriteError as ser::Error>::custom(msg))
    }
}

type ProbeResult<T = Shape> = Result<T, ProbeError>;

/// Records an object's keys without serializing any value.
struct Probe;

struct ProbeObject {
    keys: Vec<Cow<'static, str>>,
}

impl ser::Serializer for Probe {
    type Ok = Shape;
    type Error = ProbeError;
    type SerializeSeq = Impossible<Shape, ProbeError>;
    type SerializeTuple = Impossible<Shape, ProbeError>;
    type SerializeTupleStruct = Impossible<Shape, ProbeError>;
    type SerializeTupleVariant = Impossible<Shape, ProbeError>;
    type SerializeMap = ProbeObject;
    type SerializeStruct = ProbeObject;
    type SerializeStructVariant = Impossible<Shape, ProbeError>;

    fn serialize_bool(self, _v: bool) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_i8(self, _v: i8) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_i16(self, _v: i16) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_i32(self, _v: i32) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_i64(self, _v: i64) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_i128(self, _v: i128) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_u8(self, _v: u8) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_u16(self, _v: u16) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_u32(self, _v: u32) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_u64(self, _v: u64) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_u128(self, _v: u128) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_f32(self, _v: f32) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_f64(self, _v: f64) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_char(self, _v: char) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_str(self, _v: &str) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_bytes(self, _v: &[u8]) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_none(self) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> ProbeResult {
        value.serialize(self)
    }

    fn serialize_unit(self) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
    ) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> ProbeResult {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> ProbeResult {
        Ok(Shape::Other)
    }

    fn serialize_seq(self, _len: Option<usize>) -> ProbeResult<Self::SerializeSeq> {
        Err(ProbeError::Other)
    }

    fn serialize_tuple(self, _len: usize) -> ProbeResult<Self::SerializeTuple> {
        Err(ProbeError::Other)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> ProbeResult<Self::SerializeTupleStruct> {
        Err(ProbeError::Other)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> ProbeResult<Self::SerializeTupleVariant> {
        Err(ProbeError::Other)
    }

    fn serialize_map(self, len: Option<usize>) -> ProbeResult<ProbeObject> {
        Ok(ProbeObject {
            keys: Vec::with_capacity(len.unwrap_or(0)),
        })
    }

    fn serialize_struct(self, _name: &'static str, len: usize) -> ProbeResult<ProbeObject> {
        Ok(ProbeObject {
            keys: Vec::with_capacity(len),
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> ProbeResult<Self::SerializeStructVariant> {
        Err(ProbeError::Other)
    }
}

impl ser::SerializeMap for ProbeObject {
    type Ok = Shape;
    type Error = ProbeError;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> ProbeResult<()> {
        let key = key.serialize(KeyWriter).map_err(ProbeError::Real)?;
        self.keys.push(Cow::Owned(key));
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, _value: &T) -> ProbeResult<()> {
        Ok(())
    }

    fn end(self) -> ProbeResult {
        Ok(Shape::Object(self.keys))
    }
}

impl ser::SerializeStruct for ProbeObject {
    type Ok = Shape;
    type Error = ProbeError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        _value: &T,
    ) -> ProbeResult<()> {
        self.keys.push(Cow::Borrowed(key));
        Ok(())
    }

    fn end(self) -> ProbeResult {
        Ok(Shape::Object(self.keys))
    }
}

/// Writes the value of member number `target` of an object, and nothing
/// else. Returns whether that member was found.
struct Select<'a, S> {
    out: &'a mut S,
    target: usize,
}

struct SelectObject<'a, S> {
    out: &'a mut S,
    target: usize,
    index: usize,
    found: bool,
}

impl<S: Sink> SelectObject<'_, S> {
    fn member<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        if self.index == self.target {
            write_value(self.out, value, &[])?;
            self.found = true;
        }
        self.index += 1;
        Ok(())
    }
}

impl<'a, S: Sink> ser::Serializer for Select<'a, S> {
    type Ok = bool;
    type Error = WriteError;
    type SerializeSeq = Impossible<bool, WriteError>;
    type SerializeTuple = Impossible<bool, WriteError>;
    type SerializeTupleStruct = Impossible<bool, WriteError>;
    type SerializeTupleVariant = Impossible<bool, WriteError>;
    type SerializeMap = SelectObject<'a, S>;
    type SerializeStruct = SelectObject<'a, S>;
    type SerializeStructVariant = Impossible<bool, WriteError>;

    fn serialize_bool(self, _v: bool) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_i8(self, _v: i8) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_i16(self, _v: i16) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_i32(self, _v: i32) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_i64(self, _v: i64) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_u8(self, _v: u8) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_u16(self, _v: u16) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_u32(self, _v: u32) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_u64(self, _v: u64) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_f32(self, _v: f32) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_f64(self, _v: f64) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_char(self, _v: char) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_str(self, _v: &str) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_bytes(self, _v: &[u8]) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_none(self) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> WriteResult<bool> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
    ) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> WriteResult<bool> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> WriteResult<bool> {
        Err(inconsistent())
    }

    fn serialize_seq(self, _len: Option<usize>) -> WriteResult<Self::SerializeSeq> {
        Err(inconsistent())
    }

    fn serialize_tuple(self, _len: usize) -> WriteResult<Self::SerializeTuple> {
        Err(inconsistent())
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> WriteResult<Self::SerializeTupleStruct> {
        Err(inconsistent())
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> WriteResult<Self::SerializeTupleVariant> {
        Err(inconsistent())
    }

    fn serialize_map(self, _len: Option<usize>) -> WriteResult<SelectObject<'a, S>> {
        Ok(SelectObject {
            out: self.out,
            target: self.target,
            index: 0,
            found: false,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> WriteResult<SelectObject<'a, S>> {
        self.serialize_map(None)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> WriteResult<Self::SerializeStructVariant> {
        Err(inconsistent())
    }
}

impl<S: Sink> ser::SerializeMap for SelectObject<'_, S> {
    type Ok = bool;
    type Error = WriteError;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, _key: &T) -> WriteResult {
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        self.member(value)
    }

    fn end(self) -> WriteResult<bool> {
        Ok(self.found)
    }
}

impl<S: Sink> ser::SerializeStruct for SelectObject<'_, S> {
    type Ok = bool;
    type Error = WriteError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> WriteResult {
        self.member(value)
    }

    fn end(self) -> WriteResult<bool> {
        Ok(self.found)
    }
}

/// Writes one non-object value into `out`. Objects only reach it inside a
/// struct variant (or from a value that serializes differently each time);
/// they are buffered.
struct Writer<'a, S> {
    out: &'a mut S,
}

impl<'a, S: Sink> Writer<'a, S> {
    fn object(self, close: &'static str) -> Object<'a, S> {
        Object {
            out: self.out,
            members: Vec::new(),
            key: None,
            close,
        }
    }

    /// Opens `{"variant":`, closed by the compound value's `close`.
    fn open_variant(&mut self, variant: &str) -> WriteResult {
        self.out.put("{")?;
        write_string(self.out, variant)?;
        self.out.put(":")?;
        Ok(())
    }
}

impl<'a, S: Sink> ser::Serializer for Writer<'a, S> {
    type Ok = ();
    type Error = WriteError;
    type SerializeSeq = Seq<'a, S>;
    type SerializeTuple = Seq<'a, S>;
    type SerializeTupleStruct = Seq<'a, S>;
    type SerializeTupleVariant = Seq<'a, S>;
    type SerializeMap = Object<'a, S>;
    type SerializeStruct = Object<'a, S>;
    type SerializeStructVariant = Object<'a, S>;

    fn serialize_bool(self, v: bool) -> WriteResult {
        Ok(self.out.put(if v { "true" } else { "false" })?)
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
        Ok(self.out.put(&format_number(v)?)?)
    }

    fn serialize_char(self, v: char) -> WriteResult {
        Ok(write_string(self.out, v.encode_utf8(&mut [0; 4]))?)
    }

    fn serialize_str(self, v: &str) -> WriteResult {
        Ok(write_string(self.out, v)?)
    }

    fn serialize_bytes(self, v: &[u8]) -> WriteResult {
        self.out.put("[")?;
        for (i, b) in v.iter().enumerate() {
            if i > 0 {
                self.out.put(",")?;
            }
            write_u64(self.out, u64::from(*b))?;
        }
        Ok(self.out.put("]")?)
    }

    fn serialize_none(self) -> WriteResult {
        self.serialize_unit()
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> WriteResult {
        write_value(self.out, value, &[])
    }

    fn serialize_unit(self) -> WriteResult {
        Ok(self.out.put("null")?)
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
        write_value(self.out, value, &[])
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> WriteResult {
        self.open_variant(variant)?;
        write_value(self.out, value, &[])?;
        Ok(self.out.put("}")?)
    }

    fn serialize_seq(self, _len: Option<usize>) -> WriteResult<Seq<'a, S>> {
        self.out.put("[")?;
        Ok(Seq {
            out: self.out,
            first: true,
            close: "]",
        })
    }

    fn serialize_tuple(self, len: usize) -> WriteResult<Seq<'a, S>> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> WriteResult<Seq<'a, S>> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> WriteResult<Seq<'a, S>> {
        self.open_variant(variant)?;
        self.out.put("[")?;
        Ok(Seq {
            out: self.out,
            first: true,
            close: "]}",
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> WriteResult<Object<'a, S>> {
        Ok(self.object(""))
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> WriteResult<Object<'a, S>> {
        Ok(self.object(""))
    }

    fn serialize_struct_variant(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> WriteResult<Object<'a, S>> {
        self.open_variant(variant)?;
        Ok(self.object("}"))
    }
}

/// An array (or the array of a tuple variant) being written.
struct Seq<'a, S> {
    out: &'a mut S,
    first: bool,
    close: &'static str,
}

impl<S: Sink> Seq<'_, S> {
    fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        if !self.first {
            self.out.put(",")?;
        }
        self.first = false;
        write_value(self.out, value, &[])
    }

    fn finish(self) -> WriteResult {
        Ok(self.out.put(self.close)?)
    }
}

impl<S: Sink> ser::SerializeSeq for Seq<'_, S> {
    type Ok = ();
    type Error = WriteError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        self.element(value)
    }

    fn end(self) -> WriteResult {
        self.finish()
    }
}

impl<S: Sink> ser::SerializeTuple for Seq<'_, S> {
    type Ok = ();
    type Error = WriteError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        self.element(value)
    }

    fn end(self) -> WriteResult {
        self.finish()
    }
}

impl<S: Sink> ser::SerializeTupleStruct for Seq<'_, S> {
    type Ok = ();
    type Error = WriteError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        self.element(value)
    }

    fn end(self) -> WriteResult {
        self.finish()
    }
}

impl<S: Sink> ser::SerializeTupleVariant for Seq<'_, S> {
    type Ok = ();
    type Error = WriteError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> WriteResult {
        self.element(value)
    }

    fn end(self) -> WriteResult {
        self.finish()
    }
}

/// A buffered object (struct variants only): members are collected as text,
/// then sorted and written.
struct Object<'a, S> {
    out: &'a mut S,
    /// (key, canonical text of the value).
    members: Vec<(String, String)>,
    key: Option<String>,
    /// Written after the closing brace (`}` for a struct variant).
    close: &'static str,
}

impl<S: Sink> Object<'_, S> {
    fn member<T: Serialize + ?Sized>(&mut self, key: String, value: &T) -> WriteResult {
        let mut text = String::new();
        write_value(&mut text, value, &[])?;
        self.members.push((key, text));
        Ok(())
    }

    fn finish(self) -> WriteResult {
        let Object {
            out,
            mut members,
            close,
            ..
        } = self;
        // Stable, so the last of two equal keys stays last.
        members.sort_by(|a, b| utf16_cmp(&a.0, &b.0));
        out.put("{")?;
        let mut first = true;
        for (n, (key, value)) in members.iter().enumerate() {
            if members.get(n + 1).is_some_and(|next| next.0 == *key) {
                continue;
            }
            if !first {
                out.put(",")?;
            }
            first = false;
            write_string(out, key)?;
            out.put(":")?;
            out.put(value)?;
        }
        out.put("}")?;
        Ok(out.put(close)?)
    }
}

impl<S: Sink> ser::SerializeMap for Object<'_, S> {
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
        self.finish()
    }
}

impl<S: Sink> ser::SerializeStruct for Object<'_, S> {
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
        self.finish()
    }
}

impl<S: Sink> ser::SerializeStructVariant for Object<'_, S> {
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
        self.finish()
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

    /// Floats that ECMAScript prints as integer literals beyond ±(2^53 − 1)
    /// are rejected by both writers; exponent forms and safe values are not.
    #[test]
    fn floats_printed_as_unsafe_integers_are_rejected() {
        for bad in [
            9_007_199_254_740_992.0,
            1.5e16,
            1e19,
            -1e19,
            9.999_999_999_999_999e20,
        ] {
            let value = json!([bad]);
            assert!(canonicalize(&value).is_err(), "{bad}");
            assert!(to_canonical_string(&[bad]).is_err(), "{bad}");
        }
        for (ok, text) in [
            (9_007_199_254_740_991.0, "9007199254740991"),
            (1e21, "1e+21"),
            (-1e30, "-1e+30"),
            (4.5e15, "4500000000000000"),
            (0.5, "0.5"),
        ] {
            assert_eq!(canonicalize(&json!(ok)).unwrap(), text);
            assert_eq!(to_canonical_string(&ok).unwrap(), text);
            // The canonical text reads back as the same canonical text.
            let back: Value = serde_json::from_str(text).unwrap();
            assert_eq!(canonicalize(&back).unwrap(), text);
        }
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

    /// Every output path (`String`, `io::Write`, SHA-256) gives the same text as
    /// canonicalizing `serde_json::to_value`, for nested objects inside
    /// options, newtypes, variants, arrays and maps.
    #[test]
    fn streaming_paths_match_value_path() {
        use std::collections::BTreeMap;

        #[derive(Serialize)]
        struct Leaf {
            z: u8,
            a: &'static str,
            m: BTreeMap<String, Option<Leaf2>>,
        }
        #[derive(Serialize)]
        struct Leaf2 {
            y: Vec<u32>,
            b: (),
        }
        #[derive(Serialize)]
        struct Newtype(Leaf2);
        #[derive(Serialize)]
        enum Variants {
            New(Leaf2),
            Tuple(Leaf2, Option<Leaf2>),
            Struct { q: Leaf2, c: Vec<Leaf2> },
        }
        #[derive(Serialize)]
        struct Top {
            zz: Vec<Leaf>,
            opt: Option<Box<Option<Leaf2>>>,
            newtype: Newtype,
            variants: Vec<Variants>,
            value: Value,
            text: String,
        }
        let leaf2 = |n: u32| Leaf2 {
            y: (0..n).collect(),
            b: (),
        };
        let mut m = BTreeMap::new();
        m.insert("k\u{1f600}".to_owned(), Some(leaf2(2)));
        m.insert("k\u{fb33}".to_owned(), None);
        let top = Top {
            zz: vec![
                Leaf {
                    z: 1,
                    a: "x\"\\\u{1}\u{1f}é",
                    m,
                },
                Leaf {
                    z: 2,
                    a: "",
                    m: BTreeMap::new(),
                },
            ],
            opt: Some(Box::new(Some(leaf2(1)))),
            newtype: Newtype(leaf2(3)),
            variants: vec![
                Variants::New(leaf2(0)),
                Variants::Tuple(leaf2(1), None),
                Variants::Struct {
                    q: leaf2(2),
                    c: vec![leaf2(1), leaf2(0)],
                },
            ],
            value: json!({"b": [1, {"d": 2, "c": [true, null]}], "a": -0.5}),
            // Longer than the hash sink's block, with escapes across blocks.
            text: "a\n\u{7}é".repeat(40_000),
        };
        let want = canonicalize(&serde_json::to_value(&top).unwrap()).unwrap();
        assert_eq!(to_canonical_string(&top).unwrap(), want);
        let mut bytes = Vec::new();
        to_canonical_writer(&top, &mut bytes).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), want);
        assert_eq!(
            canonical_sha256_hex(&top, &[]).unwrap(),
            crate::sha256_hex(want.as_bytes())
        );

        let mut value = serde_json::to_value(&top).unwrap();
        value.as_object_mut().unwrap().remove("zz");
        let want = canonicalize(&value).unwrap();
        assert_eq!(to_canonical_string_without(&top, &["zz"]).unwrap(), want);
        let mut bytes = Vec::new();
        to_canonical_writer_without(&top, &["zz"], &mut bytes).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), want);
    }

    /// A map that repeats a key keeps the last value, like `to_value`.
    #[test]
    fn repeated_map_keys_keep_the_last_value() {
        use serde::ser::SerializeMap;

        struct Repeats;
        impl Serialize for Repeats {
            fn serialize<S: ser::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut map = s.serialize_map(None)?;
                map.serialize_entry("b", &1)?;
                map.serialize_entry("a", &json!({"y": 1, "x": 2}))?;
                map.serialize_entry("b", &[3, 4])?;
                map.end()
            }
        }
        let want = canonicalize(&serde_json::to_value(Repeats).unwrap()).unwrap();
        assert_eq!(want, r#"{"a":{"x":2,"y":1},"b":[3,4]}"#);
        assert_eq!(to_canonical_string(&Repeats).unwrap(), want);
    }

    #[test]
    fn write_errors_are_reported() {
        struct Broken;
        impl io::Write for Broken {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let err = to_canonical_writer(&json!({"a": 1}), Broken).unwrap_err();
        assert!(matches!(err, JcsError::Io(_)), "{err}");
    }
}
