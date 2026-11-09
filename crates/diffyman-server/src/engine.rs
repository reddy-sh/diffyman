//! Running the engine off the async threads.

use std::sync::Arc;

use bytes::Bytes;
use diffyman::{Input, Settings};
use serde_json::Value;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::{ApiError, OptionsProblem};

/// `provenance.binding` of every document the server produces.
pub const BINDING: &str = "server";

/// Two inputs, their names and the options of one diff.
#[derive(Clone, Debug)]
pub struct DiffInputs {
    /// Left bytes.
    pub left: Bytes,
    /// Right bytes.
    pub right: Bytes,
    /// Left name, for type detection by extension and for the document.
    pub left_name: Option<String>,
    /// Right name.
    pub right_name: Option<String>,
    /// The options object as JSON text (blank means `{}`).
    pub options: String,
}

/// Runs the engine on a blocking thread and returns the canonical JSON Diff
/// Document, byte for byte as the engine wrote it. `slot` (a semaphore
/// permit) is released when the engine returns, even if the caller stopped
/// waiting, so slots count running engine threads.
pub async fn run(
    inputs: DiffInputs,
    slot: impl Send + 'static,
) -> Result<Result<String, diffyman::Error>, ApiError> {
    tokio::task::spawn_blocking(move || {
        let result = diff(&inputs);
        drop(inputs);
        drop(slot);
        result
    })
    .await
    .map_err(|_| ApiError::internal())
}

/// The engine call itself, shared with tests.
pub fn diff(inputs: &DiffInputs) -> Result<String, diffyman::Error> {
    fn input<'a>(bytes: &'a Bytes, name: Option<&'a str>) -> Input<'a> {
        match name {
            Some(name) => Input::named(bytes, name),
            None => Input::new(bytes),
        }
    }
    diffyman::diff_inputs_with(
        input(&inputs.left, inputs.left_name.as_deref()),
        input(&inputs.right, inputs.right_name.as_deref()),
        &inputs.options,
        &Settings {
            binding: BINDING.to_owned(),
        },
    )
}

/// Memory a synchronous diff is budgeted per byte of its request body: the
/// body, the decoded inputs, the engine's working data and the document.
pub const SYNC_MEMORY_PER_BODY_BYTE: u64 = 4;

/// Admission of synchronous diffs: at most `slots` at once, and together at
/// most a memory budget, reserved by request size. A diff is admitted before
/// its body is read, and holds what it was given (an [`Admitted`]) until the
/// request is done and the engine has returned, even when the request timed
/// out: so neither the bodies being read nor the engine threads still
/// running can exceed the limits.
#[derive(Debug)]
pub struct Admission {
    slots: Arc<Semaphore>,
    memory_kib: Arc<Semaphore>,
    budget_kib: u32,
}

/// What an admitted synchronous diff holds; shared by the request and its
/// engine thread, and released when both are done. The memory they free is
/// returned to the system at once (the large blocks, through
/// [`crate::alloc::ServerAlloc`]), so the budget bounds resident memory.
#[derive(Debug)]
pub struct Admitted {
    _slot: OwnedSemaphorePermit,
    _memory: OwnedSemaphorePermit,
}

impl Admission {
    /// `slots` diffs at once (at least one) within `memory_bytes` in total.
    pub fn new(slots: u32, memory_bytes: u64) -> Admission {
        let budget_kib = u32::try_from((memory_bytes / 1024).max(1)).unwrap_or(u32::MAX);
        Admission {
            slots: Arc::new(Semaphore::new(usize::try_from(slots.max(1)).unwrap_or(1))),
            memory_kib: Arc::new(Semaphore::new(
                usize::try_from(budget_kib).unwrap_or(usize::MAX >> 4),
            )),
            budget_kib,
        }
    }

    /// Waits at most `wait` for a slot and for `estimate_bytes` of the
    /// memory budget (all of it at most, so a request larger than the budget
    /// runs alone), else answers `503 busy`.
    pub async fn admit(
        &self,
        estimate_bytes: u64,
        wait: std::time::Duration,
    ) -> Result<Arc<Admitted>, ApiError> {
        let kib = u32::try_from(estimate_bytes.div_ceil(1024))
            .unwrap_or(u32::MAX)
            .clamp(1, self.budget_kib);
        let acquire = async {
            let slot = self.slots.clone().acquire_owned().await?;
            let memory = self.memory_kib.clone().acquire_many_owned(kib).await?;
            Ok::<_, tokio::sync::AcquireError>(Admitted {
                _slot: slot,
                _memory: memory,
            })
        };
        match tokio::time::timeout(wait, acquire).await {
            Ok(Ok(admitted)) => Ok(Arc::new(admitted)),
            Ok(Err(_)) => Err(ApiError::internal()),
            Err(_) => Err(ApiError::unavailable(
                "busy",
                "every synchronous diff slot is in use; try again",
                1,
            )),
        }
    }
}

/// The fields of a document that a saved diff keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// `status`.
    pub status: String,
    /// `kind`.
    pub kind: String,
    /// `stats`, when an object.
    pub stats: Option<Value>,
}

impl Summary {
    /// Reads them from a document. Only these fields are built; the rest of
    /// the document is skipped, not parsed into memory.
    pub fn of(document: &str) -> Result<Summary, ApiError> {
        #[derive(serde::Deserialize)]
        struct Head {
            status: String,
            kind: String,
            #[serde(default)]
            stats: Option<Value>,
        }
        let head: Head = serde_json::from_str(document).map_err(|_| ApiError::internal())?;
        Ok(Summary {
            status: head.status,
            kind: head.kind,
            stats: head.stats.filter(Value::is_object),
        })
    }
}

/// Checks options JSON text as the engine and the CLI do, and returns it as
/// compact JSON text. The text goes through the engine's strict parser
/// ([`diffyman::parse_options_json`]), so an object that gives a key twice,
/// at any depth, is `invalid-options` exactly as with `diffyman diff
/// --options`; serde's `Value` would keep the last one. Missing, blank or
/// `null` options are `{}`. Error messages are fixed ([`OptionsProblem`]):
/// they never quote the options.
pub fn options_text(raw: Option<&str>) -> Result<String, ApiError> {
    let Some(text) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok("{}".to_owned());
    };
    let value = diffyman::parse_options_json(text).map_err(|error| match error {
        diffyman::Error::InvalidOptions(message) => {
            ApiError::invalid_options(match OptionsProblem::of(&message) {
                OptionsProblem::DuplicateKey => OptionsProblem::DuplicateKey,
                _ => OptionsProblem::NotJson,
            })
        }
        other => ApiError::from(other),
    })?;
    if value.is_null() {
        return Ok("{}".to_owned());
    }
    let compact = serde_json::to_string(&value).map_err(|_| ApiError::internal())?;
    diffyman::parse_options_value(value)?;
    Ok(compact)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_carry_the_server_binding() {
        let inputs = DiffInputs {
            left: Bytes::from_static(b"a\nb\n"),
            right: Bytes::from_static(b"a\nc\n"),
            left_name: Some("dir/a.txt".to_owned()),
            right_name: None,
            options: "{}".to_owned(),
        };
        let json = diff(&inputs).unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["provenance"]["binding"], "server");
        assert_eq!(value["left"]["name"], "a.txt", "final component");
        let summary = Summary::of(&json).unwrap();
        assert_eq!(summary.status, "different");
        assert_eq!(summary.kind, "text");
        assert!(summary.stats.is_some());
    }

    #[test]
    fn summaries_skip_the_rest_of_the_document() {
        let summary = Summary::of(
            r#"{"hunks":[{"x":[1,2,{"y":"z"}]}],"kind":"binary","stats":null,"status":"error"}"#,
        )
        .unwrap();
        assert_eq!(
            (summary.status.as_str(), summary.kind.as_str()),
            ("error", "binary")
        );
        assert_eq!(summary.stats, None);
        assert!(Summary::of(r#"{"kind":"text"}"#).is_err());
    }

    #[tokio::test]
    async fn admission_limits_slots_and_memory() {
        let wait = std::time::Duration::from_millis(50);
        let admission = Admission::new(2, 1 << 20);
        let first = admission.admit(600 << 10, wait).await.unwrap();
        // A slot is free, but not 600 KiB of the 1 MiB budget.
        let err = admission.admit(600 << 10, wait).await.unwrap_err();
        assert_eq!(err.code, "busy");
        let small = admission.admit(100 << 10, wait).await.unwrap();
        // Both slots are taken.
        assert_eq!(admission.admit(1, wait).await.unwrap_err().code, "busy");
        // A clone (the engine thread's) keeps the reservation alive.
        let engine_copy = Arc::clone(&first);
        drop(first);
        drop(small);
        assert_eq!(
            admission.admit(600 << 10, wait).await.unwrap_err().code,
            "busy"
        );
        drop(engine_copy);
        let again = admission.admit(600 << 10, wait).await.unwrap();
        drop(again);
        // A request over the whole budget runs alone.
        let huge = admission.admit(10 << 20, wait).await.unwrap();
        assert_eq!(admission.admit(1, wait).await.unwrap_err().code, "busy");
        drop(huge);
    }

    #[test]
    fn options_are_checked_before_queueing() {
        for empty in [None, Some(""), Some("  "), Some("null")] {
            assert_eq!(options_text(empty).unwrap(), "{}", "{empty:?}");
        }
        assert_eq!(
            options_text(Some(r#"{ "granularity": "word" }"#)).unwrap(),
            r#"{"granularity":"word"}"#
        );
        for bad in [r#"{"granularity": "unknown"}"#, r#"{"nope": 1}"#, "[1]"] {
            let error = options_text(Some(bad)).unwrap_err();
            assert_eq!(error.code, "invalid-options", "{bad}");
            assert_eq!(error.message, OptionsProblem::Invalid.message(), "{bad}");
        }
        let error = options_text(Some("{nope")).unwrap_err();
        assert_eq!(
            (error.code, error.message.as_str()),
            ("invalid-options", OptionsProblem::NotJson.message())
        );
    }

    #[test]
    fn options_that_repeat_a_key_are_refused_as_by_the_cli() {
        let repeated = [
            r#"{"context": 1, "context": 2}"#,
            r#"{"context": 1, "context": 1}"#,
            r#"{"ignore": {"case": true, "case": false}}"#,
            r#"{"tree": {"exclude": ["a"], "exclude": ["b"]}}"#,
            r#"{"ignore": {"regex": [{"x": 1, "x": 2}]}}"#,
        ];
        for text in repeated {
            // The engine (and so `diffyman diff --options`) refuses it...
            let engine = diffyman::parse_options(text).unwrap_err();
            // ...and the server with the same code and a fixed message.
            let error = options_text(Some(text)).unwrap_err();
            assert_eq!(error.code, engine.code(), "{text}");
            assert_eq!(
                error.message,
                OptionsProblem::DuplicateKey.message(),
                "{text}"
            );
        }
        // A value repeated in a list is not a repeated key.
        assert!(options_text(Some(r#"{"tree": {"exclude": ["a", "a"]}}"#)).is_ok());
    }
}
