//! The REST API (v1) and its OpenAPI document.

use axum::Router;
use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::extract::{FromRequestParts, Path};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bytes::Bytes;
use futures::TryStreamExt;
use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

use crate::error::ApiError;
use crate::guard;
use crate::state::AppState;

pub mod blobs;
pub mod diffs;
pub mod jobs;
pub mod meta;

/// Largest body of `POST /v1/jobs` (a JSON request with options).
pub const MAX_JOB_REQUEST_BYTES: u64 = 1 << 20;

/// The OpenAPI 3.1 document of the API.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "diffyman-server",
        description = "Local REST service for the diffyman diff engine. Every `/v1` route needs `Authorization: Bearer <API key>`. Diff results are canonical Diff Documents (docs/spec/diff-document.md). Errors are `{\"error\": {\"code\", \"message\"}}` with stable codes.",
        license(name = "MIT OR Apache-2.0", identifier = "MIT OR Apache-2.0"),
    ),
    paths(
        diffs::create_diff,
        diffs::get_diff,
        diffs::delete_diff,
        blobs::upload_blob,
        jobs::create_job,
        jobs::get_job,
        jobs::cancel_job,
        meta::formats,
        meta::healthz,
        meta::openapi,
    ),
    components(schemas(crate::error::ErrorBody, crate::error::ErrorDetail)),
    modifiers(&BearerAuth),
    tags(
        (name = "diffs", description = "Synchronous diffs and saved Diff Documents"),
        (name = "blobs", description = "Content-addressed uploads for jobs"),
        (name = "jobs", description = "Asynchronous diffs of uploaded blobs"),
        (name = "meta", description = "Formats, health and this document"),
    )
)]
pub struct ApiDoc;

struct BearerAuth;

impl utoipa::Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("dmk_ followed by 64 hex digits")
                    .build(),
            ),
        );
        openapi.info.contact = None;
        env!("CARGO_PKG_VERSION").clone_into(&mut openapi.info.version);
    }
}

/// The OpenAPI document as indented JSON with a final newline, as served at
/// `/openapi.json` and kept in `docs/api/openapi.json`.
pub fn openapi_json() -> String {
    let mut text = ApiDoc::openapi()
        .to_pretty_json()
        .unwrap_or_else(|_| "{}".to_owned());
    text.push('\n');
    text
}

/// The whole HTTP application.
pub fn router(state: AppState) -> Router {
    let body_limit = usize::try_from(state.config.limits.max_body_bytes).unwrap_or(usize::MAX);
    let v1 = Router::new()
        .route(
            "/v1/diff",
            post(diffs::create_diff).layer(DefaultBodyLimit::max(body_limit)),
        )
        .route(
            "/v1/diffs/{id}",
            get(diffs::get_diff).delete(diffs::delete_diff),
        )
        .route("/v1/blobs", post(blobs::upload_blob))
        .route("/v1/jobs", post(jobs::create_job))
        .route("/v1/jobs/{id}", get(jobs::get_job))
        .route("/v1/jobs/{id}/cancel", post(jobs::cancel_job))
        .route("/v1/formats", get(meta::formats))
        .route_layer(from_fn_with_state(state.clone(), guard::auth));
    Router::new()
        .route("/healthz", get(meta::healthz))
        .route("/openapi.json", get(meta::openapi))
        .merge(v1)
        .fallback(meta::not_found)
        .method_not_allowed_fallback(meta::method_not_allowed)
        .route_layer(from_fn(guard::tag_route))
        .layer(from_fn_with_state(state.clone(), guard::host_origin))
        .layer(from_fn_with_state(state.clone(), guard::context))
        .with_state(state)
}

/// An `{id}` path segment.
pub struct IdPath(pub String);

impl<S: Send + Sync> FromRequestParts<S> for IdPath {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<IdPath, ApiError> {
        let Path(id) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|_| ApiError::invalid("invalid path parameter"))?;
        Ok(IdPath(id))
    }
}

/// Reads a whole body, refusing one over `limit` bytes before and while
/// reading it.
pub async fn read_limited(headers: &HeaderMap, body: Body, limit: u64) -> Result<Bytes, ApiError> {
    let too_large = || {
        ApiError::too_large(
            "payload-too-large",
            format!("the request body is larger than {limit} bytes"),
        )
    };
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > limit) {
        return Err(too_large());
    }
    let mut stream = body.into_data_stream();
    let mut out = Vec::with_capacity(declared.map_or(0, |n| usize::try_from(n).unwrap_or(0)));
    while let Some(chunk) = stream.try_next().await.map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "input-error",
            "reading the request body failed",
        )
    })? {
        if (out.len() + chunk.len()) as u64 > limit {
            return Err(too_large());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(out))
}

/// The media type of a request, lowercase, without parameters.
pub fn media_type(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::CONTENT_TYPE)?.to_str().ok()?;
    let essence = value.split(';').next().unwrap_or("").trim();
    (!essence.is_empty()).then(|| essence.to_ascii_lowercase())
}

/// A query parameter, or `None`. Repeated parameters are refused.
pub fn query_param(raw: Option<&str>, name: &str) -> Result<Option<String>, ApiError> {
    let mut found = None;
    for (key, value) in url::form_urlencoded::parse(raw.unwrap_or("").as_bytes()) {
        if key == name {
            if found.is_some() {
                return Err(ApiError::invalid(format!("`{name}` is given twice")));
            }
            found = Some(value.into_owned());
        }
    }
    Ok(found)
}

/// A boolean query parameter (`true`, `false`, `1`, `0`), `false` when
/// absent.
pub fn query_flag(raw: Option<&str>, name: &str) -> Result<bool, ApiError> {
    match query_param(raw, name)?.as_deref() {
        None | Some("false" | "0") => Ok(false),
        Some("true" | "1") => Ok(true),
        Some(_) => Err(ApiError::invalid(format!("`{name}` must be true or false"))),
    }
}

/// A JSON response from text that is already JSON.
pub fn json_text(status: StatusCode, text: String) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], text).into_response()
}

/// Parses a JSON request body into `T`.
pub fn parse_json<T: serde::de::DeserializeOwned>(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<T, ApiError> {
    if media_type(headers).as_deref() != Some("application/json") {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported-media-type",
            "send Content-Type: application/json",
        ));
    }
    serde_json::from_slice(body).map_err(|e| json_error(&e))
}

/// A body that does not parse, as a fixed message with the position and
/// serde's category. serde's own text is never used: it quotes the values
/// and field names it saw, which are request content.
fn json_error(error: &serde_json::Error) -> ApiError {
    use serde_json::error::Category;
    let (category, meaning) = match error.classify() {
        Category::Syntax => ("syntax", "the body is not well-formed JSON"),
        Category::Eof => ("eof", "the body ends too early"),
        Category::Data => ("data", "a field is unknown, missing or of the wrong type"),
        Category::Io => ("io", "the body could not be read"),
    };
    ApiError::invalid(format!(
        "invalid JSON body: {category} error at line {}, column {}: {meaning}",
        error.line(),
        error.column()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_parameters() {
        assert!(!query_flag(None, "save").unwrap());
        assert!(query_flag(Some("save=true"), "save").unwrap());
        assert!(query_flag(Some("x=1&save=1"), "save").unwrap());
        assert!(!query_flag(Some("save=0"), "save").unwrap());
        assert!(query_flag(Some("save=yes"), "save").is_err());
        assert!(query_flag(Some("save=1&save=0"), "save").is_err());
        assert_eq!(
            query_param(Some("sha256=ab%63"), "sha256")
                .unwrap()
                .as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn invalid_json_errors_never_quote_the_body() {
        #[derive(Debug, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct Shape {
            count: u32,
        }
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        let bodies: [(&[u8], &str); 4] = [
            (
                br#"{"count": "MARKER-7f3a"}"#,
                "data error at line 1, column 23",
            ),
            (br#"{"MARKER-7f3a": 1}"#, "data error at line 1"),
            (b"{\"count\": MARKER-7f3a}", "syntax error at line 1"),
            (br#"{"count": "MARKER-7f3a"#, "eof error at line 1"),
        ];
        for (body, expected) in bodies {
            let error = parse_json::<Shape>(&headers, body).unwrap_err();
            assert_eq!(error.code, "invalid-request");
            assert!(!error.message.contains("MARKER"), "{}", error.message);
            assert!(error.message.contains(expected), "{}", error.message);
        }
    }

    #[test]
    fn media_types_drop_parameters() {
        let mut headers = HeaderMap::new();
        assert_eq!(media_type(&headers), None);
        headers.insert(
            header::CONTENT_TYPE,
            "Application/JSON; charset=utf-8".parse().unwrap(),
        );
        assert_eq!(media_type(&headers).as_deref(), Some("application/json"));
    }
}
