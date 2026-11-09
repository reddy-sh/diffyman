//! `diffyman-java`: the JNI library behind the `sh.reddy.diffyman` Java package.
//!
//! The native layer is deliberately thin (`docs/adr/0007-binding-strategy.md`):
//! bytes and an options JSON string go in, the canonical (RFC 8785) Diff
//! Document comes out. It is returned as UTF-8 bytes, which Java decodes into
//! a `String`, so no text passes through JNI's modified UTF-8. Path reading,
//! null checks and argument rules live in `Diffyman.java`.
//!
//! When no document can be produced, the methods throw
//! `sh.reddy.diffyman.DiffymanException` with the engine's code
//! (`invalid-options`, `io`, `unsupported`, `canonical`) and message. A JNI
//! failure or a panic becomes a `java.lang.RuntimeException` instead of
//! unwinding into the JVM.
//!
//! Every method copies its arrays and strings out of the JVM before the diff
//! starts and holds no JNI critical section or monitor while it runs, so calls
//! from many threads run in parallel.

use std::path::Path;

use diffyman::{Input, Settings};
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JByteArray, JClass, JString, JThrowable};
use jni::sys::jlong;
use jni::{Env, EnvUnowned, jni_sig, jni_str};

/// The surface name written to `provenance.binding`.
pub const BINDING: &str = "java";

/// Diffs two byte strings and returns the canonical JSON Diff Document.
///
/// `options_json` is an options object as JSON text; blank text means `{}`.
/// Names become `left.name` and `right.name`; only the final path component
/// is kept.
pub fn diff_json(
    left: &[u8],
    right: &[u8],
    options_json: &str,
    left_name: Option<&str>,
    right_name: Option<&str>,
) -> Result<String, diffyman::Error> {
    diffyman::diff_inputs_with(
        input(left, left_name),
        input(right, right_name),
        options_json,
        &settings(),
    )
}

/// Diffs two paths (files, folders or archives) and returns the canonical
/// JSON Diff Document, as `diffyman diff --json` does.
///
/// `limit` is the read limit in bytes for every input that is read whole (the
/// CLI's `--max-input-size`); `None` keeps the default rule of
/// [`diffyman::read_path`].
pub fn diff_paths_json(
    left: &Path,
    right: &Path,
    options_json: &str,
    limit: Option<u64>,
) -> Result<String, diffyman::Error> {
    let options = diffyman::parse_options(options_json)?;
    let doc = diffyman::diff_paths_with_limit(left, right, &options, &settings(), limit)?;
    Ok(diffyman::model::to_canonical_json(&doc)?)
}

fn settings() -> Settings {
    Settings {
        binding: BINDING.to_owned(),
    }
}

fn input<'a>(bytes: &'a [u8], name: Option<&'a str>) -> Input<'a> {
    match name {
        Some(name) => Input::named(bytes, name),
        None => Input::new(bytes),
    }
}

/// A Java `String` that may be `null`.
fn optional_string(env: &Env<'_>, value: &JString<'_>) -> jni::errors::Result<Option<String>> {
    if value.is_null() {
        Ok(None)
    } else {
        value.try_to_string(env).map(Some)
    }
}

/// Returns the document as a Java `byte[]`, or throws `DiffymanException`.
fn finish<'local>(
    env: &mut Env<'local>,
    result: Result<String, diffyman::Error>,
) -> jni::errors::Result<JByteArray<'local>> {
    match result {
        Ok(json) => env.byte_array_from_slice(json.as_bytes()),
        Err(error) => {
            let code = JString::from_str(env, error.code())?;
            let message = JString::from_str(env, error.to_string())?;
            let exception = env.new_object(
                jni_str!("sh/reddy/diffyman/DiffymanException"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"),
                &[(&code).into(), (&message).into()],
            )?;
            let exception = env.cast_local::<JThrowable<'_>>(exception)?;
            // `throw` reports the now pending exception as an error, which
            // `ThrowRuntimeExAndDefault` leaves in place (returning `null`).
            env.throw(exception)?;
            Ok(JByteArray::default())
        }
    }
}

/// `NativeLibrary.diff(byte[], byte[], String, String, String)`: the
/// document as UTF-8 bytes. The Java caller passes non-null arrays and
/// options; the names may be `null`.
#[allow(
    unsafe_code,
    reason = "JNI looks native methods up by their exported, unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_sh_reddy_diffyman_NativeLibrary_diff<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    left: JByteArray<'local>,
    right: JByteArray<'local>,
    options_json: JString<'local>,
    left_name: JString<'local>,
    right_name: JString<'local>,
) -> JByteArray<'local> {
    unowned_env
        .with_env(|env| -> jni::errors::Result<JByteArray<'local>> {
            let left = env.convert_byte_array(&left)?;
            let right = env.convert_byte_array(&right)?;
            let options_json = options_json.try_to_string(env)?;
            let left_name = optional_string(env, &left_name)?;
            let right_name = optional_string(env, &right_name)?;
            let result = diff_json(
                &left,
                &right,
                &options_json,
                left_name.as_deref(),
                right_name.as_deref(),
            );
            finish(env, result)
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `NativeLibrary.diffPaths(String, String, String, long)`: the document as
/// UTF-8 bytes. A negative `limit` means no limit beyond the default rule.
#[allow(
    unsafe_code,
    reason = "JNI looks native methods up by their exported, unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_sh_reddy_diffyman_NativeLibrary_diffPaths<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    left: JString<'local>,
    right: JString<'local>,
    options_json: JString<'local>,
    limit: jlong,
) -> JByteArray<'local> {
    unowned_env
        .with_env(|env| -> jni::errors::Result<JByteArray<'local>> {
            let left = left.try_to_string(env)?;
            let right = right.try_to_string(env)?;
            let options_json = options_json.try_to_string(env)?;
            let limit = u64::try_from(limit).ok();
            let result = diff_paths_json(Path::new(&left), Path::new(&right), &options_json, limit);
            finish(env, result)
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `NativeLibrary.engineVersion()`: the engine version, such as `0.0.1`.
#[allow(
    unsafe_code,
    reason = "JNI looks native methods up by their exported, unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_sh_reddy_diffyman_NativeLibrary_engineVersion<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| JString::from_str(env, diffyman::ENGINE_VERSION))
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `NativeLibrary.schemaVersion()`: the Diff Document schema version this
/// build writes, such as `0.4.0`.
#[allow(
    unsafe_code,
    reason = "JNI looks native methods up by their exported, unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_sh_reddy_diffyman_NativeLibrary_schemaVersion<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| JString::from_str(env, diffyman::model::SCHEMA_VERSION))
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `NativeLibrary.documentSchema()`: the Diff Document's JSON Schema as UTF-8
/// JSON text, the same text `diffyman schema` prints without its final
/// newline.
#[allow(
    unsafe_code,
    reason = "JNI looks native methods up by their exported, unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_sh_reddy_diffyman_NativeLibrary_documentSchema<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> JByteArray<'local> {
    unowned_env
        .with_env(|env| {
            let schema = format!("{:#}", diffyman::model::document_schema());
            env.byte_array_from_slice(schema.as_bytes())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_canonical_json_with_java_provenance() {
        let json = diff_json(
            b"a\nb\n",
            b"a\nc\n",
            r#"{"context": 1}"#,
            Some("left.txt"),
            Some("dir/right.txt"),
        )
        .unwrap();
        assert!(json.contains(r#""status":"different""#));
        assert!(json.contains(r#""binding":"java""#));
        assert!(json.contains(r#""name":"right.txt""#));
    }

    #[test]
    fn invalid_options_are_an_error() {
        let err = diff_json(b"", b"", r#"{"bogus": true}"#, None, None).unwrap_err();
        assert_eq!(err.code(), "invalid-options");
        assert!(err.to_string().starts_with("invalid options"), "{err:?}");
    }

    #[test]
    fn missing_paths_are_an_io_error() {
        let err = diff_paths_json(
            Path::new("does-not-exist-left"),
            Path::new("does-not-exist-right"),
            "",
            None,
        )
        .unwrap_err();
        assert_eq!(err.code(), "io");
    }
}
