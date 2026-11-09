//! Typed JSON errors: `{"error": {"code": "...", "message": "..."}}`.
//!
//! Codes are stable kebab-case strings; clients branch on the code, never on
//! the message. Messages never contain file content, file names, paths or
//! secrets.
//!
//! | Status | Codes |
//! |---|---|
//! | 400 | `invalid-request`, `invalid-options`, `invalid-blob-id`, `hash-mismatch`, `input-error`, `nul-character` |
//! | 401 | `unauthorized` |
//! | 403 | `forbidden`, `host-not-allowed`, `origin-not-allowed` |
//! | 404 | `not-found`, `blob-not-found`, `job-not-found`, `diff-not-found` |
//! | 405 | `method-not-allowed` |
//! | 409 | `job-finished` |
//! | 413 | `payload-too-large`, `input-too-large`, `blob-too-large`, `job-too-large` |
//! | 415 | `unsupported-media-type` |
//! | 422 | `unsupported` |
//! | 429 | `rate-limited` |
//! | 431 | `headers-too-large` |
//! | 500 | `internal` |
//! | 503 | `db-unavailable`, `storage-unavailable`, `storage-conflict`, `busy`, `timeout` |

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use diffyman_db::DbError;
use diffyman_storage::StorageError;
use serde::Serialize;
use utoipa::ToSchema;

/// Every error code the API returns, with its HTTP status.
pub const CODES: &[(&str, u16)] = &[
    ("invalid-request", 400),
    ("invalid-options", 400),
    ("invalid-blob-id", 400),
    ("hash-mismatch", 400),
    ("input-error", 400),
    ("nul-character", 400),
    ("unauthorized", 401),
    ("forbidden", 403),
    ("host-not-allowed", 403),
    ("origin-not-allowed", 403),
    ("not-found", 404),
    ("blob-not-found", 404),
    ("job-not-found", 404),
    ("diff-not-found", 404),
    ("method-not-allowed", 405),
    ("job-finished", 409),
    ("payload-too-large", 413),
    ("input-too-large", 413),
    ("blob-too-large", 413),
    ("job-too-large", 413),
    ("unsupported-media-type", 415),
    ("unsupported", 422),
    ("rate-limited", 429),
    ("headers-too-large", 431),
    ("internal", 500),
    ("db-unavailable", 503),
    ("storage-unavailable", 503),
    ("storage-conflict", 503),
    ("busy", 503),
    ("timeout", 503),
];

/// An API error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    /// HTTP status.
    pub status: StatusCode,
    /// Stable code, one of [`CODES`].
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
    /// Seconds to wait before retrying (`Retry-After`), for 429 and 503.
    pub retry_after: Option<u64>,
}

/// The error code of a response, for usage records and request logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErrorCode(pub &'static str);

/// Body of every error response.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    /// The error.
    pub error: ErrorDetail,
}

/// An error code and message.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorDetail {
    /// Stable kebab-case code, such as `unauthorized` or `input-too-large`.
    /// New codes may be added; treat unknown ones by their HTTP status.
    #[schema(example = "input-too-large")]
    pub code: String,
    /// Explanation for people; never parse it.
    pub message: String,
}

impl ApiError {
    /// An error with a status, code and message. The status of a code in
    /// [`CODES`] is the one listed there.
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> ApiError {
        debug_assert!(
            CODES
                .iter()
                .any(|(c, s)| *c == code && *s == status.as_u16()),
            "{code} with {status}"
        );
        ApiError {
            status,
            code,
            message: message.into(),
            retry_after: None,
        }
    }

    /// 400 `invalid-request`.
    pub fn invalid(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, "invalid-request", message)
    }

    /// 400 `nul-character`: `what` (such as "`left.name`") holds a NUL
    /// character (U+0000), which the database cannot store. Names never get
    /// into the message, only the field.
    pub fn nul_character(what: &str) -> ApiError {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "nul-character",
            format!("{what} must not contain a NUL character (U+0000)"),
        )
    }

    /// 404 with a specific code.
    pub fn not_found(code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, code, message)
    }

    /// 413 with a specific code.
    pub fn too_large(code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, code, message)
    }

    /// 503 with a specific code and `Retry-After`.
    pub fn unavailable(code: &'static str, message: impl Into<String>, retry: u64) -> ApiError {
        ApiError {
            retry_after: Some(retry),
            ..ApiError::new(StatusCode::SERVICE_UNAVAILABLE, code, message)
        }
    }

    /// 500 `internal`, with no detail.
    pub fn internal() -> ApiError {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code.to_owned(),
                message: self.message,
            },
        };
        let mut response = (self.status, Json(body)).into_response();
        let headers = response.headers_mut();
        if self.status == StatusCode::UNAUTHORIZED {
            headers.insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        if let Some(secs) = self.retry_after {
            headers.insert(header::RETRY_AFTER, HeaderValue::from(secs));
        }
        response.extensions_mut().insert(ErrorCode(self.code));
        response
    }
}

impl From<DbError> for ApiError {
    fn from(error: DbError) -> ApiError {
        match error {
            DbError::Unavailable(_) => {
                ApiError::unavailable("db-unavailable", "the database is unavailable", 5)
            }
            DbError::NotFound => ApiError::not_found("not-found", "record not found"),
            DbError::MissingReference => {
                ApiError::not_found("blob-not-found", "a referenced blob does not exist")
            }
            DbError::InvalidValue => ApiError::invalid("a value was rejected by the schema"),
            DbError::InvalidText => ApiError::new(
                StatusCode::BAD_REQUEST,
                "nul-character",
                "a value contains a NUL character (U+0000), which cannot be stored",
            ),
            _ => ApiError::internal(),
        }
    }
}

impl From<StorageError> for ApiError {
    fn from(error: StorageError) -> ApiError {
        let message = error.to_string();
        match error {
            StorageError::InvalidId => {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid-blob-id", message)
            }
            StorageError::NotFound => ApiError::not_found("blob-not-found", message),
            StorageError::TooLarge { .. } => ApiError::too_large("blob-too-large", message),
            StorageError::HashMismatch => {
                ApiError::new(StatusCode::BAD_REQUEST, "hash-mismatch", message)
            }
            StorageError::Input(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                "input-error",
                "reading the request body failed",
            ),
            StorageError::Backend(_) | StorageError::InvalidRange => {
                ApiError::unavailable("storage-unavailable", "the blob store is unavailable", 5)
            }
            StorageError::InvalidUrl(_) => ApiError::internal(),
        }
    }
}

/// Why an options object is invalid, as far as an error message may say:
/// the engine's own message quotes the request (an unknown or repeated key,
/// a value of the wrong type), so only these fixed texts are sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionsProblem {
    /// The options text is not JSON (a multipart `options` field).
    NotJson,
    /// An object in the options gives one key twice (spec §9).
    DuplicateKey,
    /// Anything else: an unknown key, a wrong type, an unknown value, a
    /// number out of range.
    Invalid,
}

impl OptionsProblem {
    /// The problem of an engine options error. The engine's message of a
    /// repeated key starts with `duplicate key` (`diffyman::parse_options`).
    pub fn of(engine_message: &str) -> OptionsProblem {
        if engine_message.starts_with("duplicate key") {
            OptionsProblem::DuplicateKey
        } else {
            OptionsProblem::Invalid
        }
    }

    /// The fixed message.
    pub fn message(self) -> &'static str {
        match self {
            OptionsProblem::NotJson => "invalid options: the options are not valid JSON",
            OptionsProblem::DuplicateKey => {
                "invalid options: an object in the options gives a key twice"
            }
            OptionsProblem::Invalid => {
                "invalid options: an option is unknown, of the wrong type or out of range"
            }
        }
    }
}

impl ApiError {
    /// 400 `invalid-options` with the fixed message of `problem`.
    pub fn invalid_options(problem: OptionsProblem) -> ApiError {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid-options",
            problem.message(),
        )
    }
}

impl From<diffyman::Error> for ApiError {
    fn from(error: diffyman::Error) -> ApiError {
        match error {
            diffyman::Error::InvalidOptions(message) => {
                ApiError::invalid_options(OptionsProblem::of(&message))
            }
            diffyman::Error::Unsupported(message) => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "unsupported",
                format!("unsupported: {message}"),
            ),
            diffyman::Error::Canonical(_) | diffyman::Error::Io(_) => ApiError::internal(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_kebab_case_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for (code, status) in CODES {
            assert!(seen.insert(code), "{code} twice");
            assert!(
                code.split('-')
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase())),
                "{code}"
            );
            assert!(StatusCode::from_u16(*status).is_ok());
        }
    }

    #[test]
    fn errors_render_as_typed_json() {
        let response = ApiError::unavailable("busy", "try again", 2).into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()[header::RETRY_AFTER], "2");
        assert_eq!(
            response.extensions().get::<ErrorCode>(),
            Some(&ErrorCode("busy"))
        );
        let response =
            ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", "no").into_response();
        assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
    }

    #[test]
    fn nul_in_a_stored_value_is_a_typed_400() {
        let error: ApiError = DbError::InvalidText.into();
        assert_eq!(
            (error.status, error.code),
            (StatusCode::BAD_REQUEST, "nul-character")
        );
        let error = ApiError::nul_character("`left.name`");
        assert_eq!(error.code, "nul-character");
        assert!(
            error.message.starts_with("`left.name`"),
            "{}",
            error.message
        );
    }

    #[test]
    fn invalid_options_messages_are_fixed() {
        // The engine's messages quote the request: an unknown key, a repeated
        // key, a value of the wrong type.
        let marker = "MARKER-5d1e";
        let cases = [
            (format!("{{\"{marker}\": 1}}"), OptionsProblem::Invalid),
            (
                format!("{{\"granularity\": \"{marker}\"}}"),
                OptionsProblem::Invalid,
            ),
            (
                format!("{{\"context\": \"{marker}\"}}"),
                OptionsProblem::Invalid,
            ),
            (
                format!("{{\"{marker}\": 1, \"{marker}\": 2}}"),
                OptionsProblem::DuplicateKey,
            ),
            (
                r#"{"ignore": {"case": true, "case": false}}"#.to_owned(),
                OptionsProblem::DuplicateKey,
            ),
        ];
        for (text, problem) in cases {
            let engine = diffyman::parse_options(&text).unwrap_err();
            assert_eq!(engine.code(), "invalid-options");
            let error = ApiError::from(engine);
            assert_eq!(
                (error.status, error.code),
                (StatusCode::BAD_REQUEST, "invalid-options"),
                "{text}"
            );
            assert_eq!(error.message, problem.message(), "{text}");
            assert!(!error.message.contains(marker), "{}", error.message);
        }
    }

    #[test]
    fn backend_errors_hide_their_source() {
        let source = std::io::Error::other("/secret/path/blobs/ab/cd");
        let error: ApiError = StorageError::Backend(Box::new(source)).into();
        assert_eq!(error.code, "storage-unavailable");
        assert!(!error.message.contains("secret"));
        let error: ApiError = StorageError::Input(std::io::Error::other("/x/y")).into();
        assert!(!error.message.contains("/x/y"));
    }
}
