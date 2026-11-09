//! `diffyman._native`: the `PyO3` extension behind the `diffyman` Python package.
//!
//! The native layer is deliberately thin: bytes and an options JSON string go
//! in, the canonical (RFC 8785) Diff Document comes out as a `str`. Input
//! coercion (paths, `str`, `dict` options) lives in `python/diffyman/__init__.py`.
//! See `docs/adr/0007-binding-strategy.md`.

use diffyman::{Error, Input, Settings};
use pyo3::create_exception;
use pyo3::exceptions::{PyOSError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

create_exception!(
    diffyman,
    DiffymanError,
    PyValueError,
    "Raised when the options are invalid (unknown key, wrong type or unknown value)."
);

/// Diffs two byte strings and returns the canonical JSON Diff Document.
///
/// `options_json` is an options object as JSON text; blank text means `{}`.
/// `left_name` and `right_name` become `left.name` and `right.name`; only the
/// final path component is kept. The GIL is released while the diff runs.
#[pyfunction]
#[pyo3(signature = (left, right, options_json = "", left_name = None, right_name = None))]
fn diff(
    py: Python<'_>,
    left: &[u8],
    right: &[u8],
    options_json: &str,
    left_name: Option<&str>,
    right_name: Option<&str>,
) -> PyResult<String> {
    let settings = Settings {
        binding: "python".to_owned(),
    };
    let result = py.detach(|| {
        diffyman::diff_inputs_with(
            input(left, left_name),
            input(right, right_name),
            options_json,
            &settings,
        )
    });
    result.map_err(|e| to_py_error(&e))
}

/// Diffs two paths and returns the canonical JSON Diff Document.
///
/// A folder on either side gives a tree document (folders and archives, spec
/// §19); two files are diffed like [`diff`], named by their final path
/// components. The GIL is released while the diff runs.
///
/// `limit` is the read limit in bytes for every input that is read whole (as
/// the CLI's `--max-input-size`); `None` keeps the default rule of
/// [`diffyman::read_path`].
#[pyfunction]
#[pyo3(signature = (left, right, options_json = "", limit = None))]
fn diff_paths(
    py: Python<'_>,
    left: std::path::PathBuf,
    right: std::path::PathBuf,
    options_json: &str,
    limit: Option<u64>,
) -> PyResult<String> {
    let settings = Settings {
        binding: "python".to_owned(),
    };
    let result = py.detach(move || {
        let options = diffyman::parse_options(options_json)?;
        let doc = diffyman::diff_paths_with_limit(&left, &right, &options, &settings, limit)?;
        Ok::<_, Error>(diffyman::model::to_canonical_json(&doc)?)
    });
    result.map_err(|e| to_py_error(&e))
}

/// Reads one file with the engine's bounded reader (spec §19.1, reading
/// paths): a regular file whole, a pipe or device up to
/// [`diffyman::MAX_STREAM_BYTES`], and with `limit` set never more than
/// `limit` bytes. Over the limit raises `OSError` (`limit-exceeded: ...`).
/// The GIL is released while the file is read.
#[pyfunction]
#[pyo3(signature = (path, limit = None))]
fn read_path(
    py: Python<'_>,
    path: std::path::PathBuf,
    limit: Option<u64>,
) -> PyResult<Py<PyBytes>> {
    let data = py
        .detach(move || diffyman::read_path(&path, limit))
        .map_err(|e| to_py_error(&e))?;
    Ok(PyBytes::new(py, &data).unbind())
}

/// Returns the JSON Schema (2020-12) of the Diff Document as pretty-printed
/// JSON text: the same text `diffyman schema` prints, without its final
/// newline. `tools/gen_types.py` generates `diffyman.types` from it.
#[pyfunction]
fn document_schema() -> String {
    format!("{:#}", diffyman::model::document_schema())
}

fn to_py_error(e: &Error) -> PyErr {
    match e {
        // `Unsupported` needs a build without archive support; this binding
        // always has it.
        Error::InvalidOptions(_) | Error::Unsupported(_) => DiffymanError::new_err(e.to_string()),
        Error::Io(_) => PyOSError::new_err(e.to_string()),
        Error::Canonical(_) => PyRuntimeError::new_err(e.to_string()),
    }
}

fn input<'a>(bytes: &'a [u8], name: Option<&'a str>) -> Input<'a> {
    match name {
        Some(name) => Input::named(bytes, name),
        None => Input::new(bytes),
    }
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(diff, m)?)?;
    m.add_function(wrap_pyfunction!(diff_paths, m)?)?;
    m.add_function(wrap_pyfunction!(read_path, m)?)?;
    m.add_function(wrap_pyfunction!(document_schema, m)?)?;
    m.add("DiffymanError", m.py().get_type::<DiffymanError>())?;
    m.add("MAX_STREAM_BYTES", diffyman::MAX_STREAM_BYTES)?;
    m.add("ENGINE_VERSION", diffyman::ENGINE_VERSION)?;
    m.add("SCHEMA_VERSION", diffyman::model::SCHEMA_VERSION)?;
    Ok(())
}
