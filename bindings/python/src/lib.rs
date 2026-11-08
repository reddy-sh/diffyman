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
#[pyfunction]
#[pyo3(signature = (left, right, options_json = ""))]
fn diff_paths(
    py: Python<'_>,
    left: std::path::PathBuf,
    right: std::path::PathBuf,
    options_json: &str,
) -> PyResult<String> {
    let settings = Settings {
        binding: "python".to_owned(),
    };
    let result = py.detach(move || {
        let options = diffyman::parse_options(options_json)?;
        let doc = diffyman::diff_paths(&left, &right, &options, &settings)?;
        Ok::<_, Error>(diffyman::model::to_canonical_json(&doc)?)
    });
    result.map_err(|e| to_py_error(&e))
}

fn to_py_error(e: &Error) -> PyErr {
    match e {
        Error::InvalidOptions(_) => DiffymanError::new_err(e.to_string()),
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
    m.add("DiffymanError", m.py().get_type::<DiffymanError>())?;
    m.add("ENGINE_VERSION", diffyman::ENGINE_VERSION)?;
    m.add("SCHEMA_VERSION", diffyman::model::SCHEMA_VERSION)?;
    Ok(())
}
