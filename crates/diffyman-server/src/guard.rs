//! Middleware: request context, `Host` and `Origin` checks, authentication.
//!
//! Order, outermost first: [`context`] (request ID, time limit, small bodies
//! read ahead, one log line), [`host_origin`] (DNS-rebinding and
//! cross-origin defence, CORS for allow-listed origins), routing,
//! [`tag_route`], and on `/v1` routes [`auth`] (API key, scope, rate limit,
//! usage record).

use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use diffyman_db::api_keys::{self, ApiKey};
use diffyman_db::records::{self, Usage};
use uuid::Uuid;

use crate::config::AuthMode;
use crate::error::{ApiError, ErrorCode};
use crate::state::AppState;

/// The route template that served a request, such as `/v1/jobs/{id}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteName(pub String);

/// Who is calling: the API key, or nobody when authentication is off.
#[derive(Clone, Debug)]
pub struct Caller {
    /// The key, under `api_key` authentication.
    pub key: Option<ApiKey>,
}

impl Caller {
    /// The key's ID; records it created carry this ID, and only it sees them.
    pub fn key_id(&self) -> Option<Uuid> {
        self.key.as_ref().map(|k| k.id)
    }

    /// Whether a record created by `owner` is visible to this caller.
    pub fn owns(&self, owner: Option<Uuid>) -> bool {
        self.key_id() == owner
    }
}

/// Response headers that allow-listed browser origins may read.
const EXPOSED: &str = "location, retry-after, x-request-id";

fn content_length(headers: &HeaderMap) -> u64 {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Bytes in a response body: its `Content-Length`, or the exact size of an
/// in-memory body (0 for a stream of unknown length).
fn body_len(response: &Response) -> u64 {
    use axum::body::HttpBody;
    match content_length(response.headers()) {
        0 => response.body().size_hint().exact().unwrap_or(0),
        n => n,
    }
}

fn millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Request bodies of a declared length up to this many bytes are read
/// before the request is handled (see [`read_small_body`]).
const READ_AHEAD_BYTES: u64 = 64 * 1024;

/// Reads a small body of declared length (at most [`READ_AHEAD_BYTES`])
/// before the request is handled, and hands the handler the bytes.
///
/// A request can be answered without its body being read: a refused method
/// or `Host`, a missing key, an invalid query. hyper then keeps the
/// connection only when the whole body had already arrived with the head;
/// when the body comes a moment later (clients often send the head and the
/// body in separate writes), it closes the connection after the answer,
/// without `Connection: close`, and the client's next request on it fails.
/// Read here, the body never decides whether the connection stays open.
/// Larger bodies and chunked ones of unknown length are left to the
/// handler, as before: a refusal does not wait for them, and the connection
/// may close after it. A read error is handed on, so the handler answers it
/// as it would have.
async fn read_small_body(request: Request) -> Request {
    use axum::body::{Body, HttpBody};

    let declared = request.body().size_hint().exact();
    if !declared.is_some_and(|n| (1..=READ_AHEAD_BYTES).contains(&n)) {
        return request;
    }
    let (parts, body) = request.into_parts();
    let limit = usize::try_from(READ_AHEAD_BYTES).unwrap_or(usize::MAX);
    let body = match axum::body::to_bytes(body, limit).await {
        Ok(bytes) => Body::from(bytes),
        Err(error) => Body::from_stream(futures::stream::once(async move {
            Err::<axum::body::Bytes, _>(error)
        })),
    };
    Request::from_parts(parts, body)
}

/// Request ID, the request time limit and the request log line.
pub async fn context(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let request_id = Uuid::new_v4().simple().to_string()[..16].to_owned();
    let bytes_in = content_length(request.headers());
    let limit = state.config.limits.request_timeout;
    // Within the time limit: a small body sent slowly ends with `timeout`.
    let handled = async move { next.run(read_small_body(request).await).await };
    let mut response = match tokio::time::timeout(limit, handled).await {
        Ok(response) => response,
        Err(_) => ApiError::unavailable(
            "timeout",
            format!(
                "the request took longer than its limit of {} s",
                limit.as_secs()
            ),
            1,
        )
        .into_response(),
    };
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    let route = response
        .extensions()
        .get::<RouteName>()
        .map_or("unmatched", |r| r.0.as_str());
    let code = response.extensions().get::<ErrorCode>().map(|c| c.0);
    tracing::info!(
        request_id = request_id.as_str(),
        method = method.as_str(),
        route,
        status = response.status().as_u16(),
        code,
        bytes_in,
        bytes_out = body_len(&response),
        duration_ms = millis(started),
        "request"
    );
    response
}

/// Copies the matched route template into the response, for the log line.
pub async fn tag_route(request: Request, next: Next) -> Response {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_owned());
    let mut response = next.run(request).await;
    if let Some(route) = route {
        response.extensions_mut().insert(RouteName(route));
    }
    response
}

/// The `Host` the request names: the authority of an absolute-form target
/// or the `Host` header, lowercase. Both must agree when both are present.
fn request_host(request: &Request) -> Option<String> {
    let header = request
        .headers()
        .get(header::HOST)
        .map(|h| h.to_str().map(str::to_ascii_lowercase));
    let header = match header {
        Some(Ok(text)) => Some(text),
        Some(Err(_)) => return None,
        None => None,
    };
    let authority = request
        .uri()
        .authority()
        .map(|a| a.as_str().to_ascii_lowercase());
    match (authority, header) {
        (Some(a), Some(h)) if a != h => None,
        (Some(a), _) => Some(a),
        (None, h) => h,
    }
}

/// Refuses requests whose `Host` is not allow-listed (a page on another
/// site that resolves its own name to 127.0.0.1 sends its own name) and
/// requests with an `Origin` that is not allow-listed (any page in a
/// browser). Allow-listed origins get CORS headers and preflight answers.
pub async fn host_origin(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let config = &state.config;
    // Two `Host` headers name two hosts; checking only one of them would let
    // the other through to whatever reads it (RFC 9112 §3.2: answer 400).
    if request
        .headers()
        .get_all(header::HOST)
        .iter()
        .nth(1)
        .is_some()
    {
        return ApiError::invalid("send exactly one Host header").into_response();
    }
    let host_ok = request_host(&request).is_some_and(|h| config.allowed_hosts.contains(&h));
    if !host_ok {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            "host-not-allowed",
            "the Host header is not one this server answers to",
        )
        .into_response();
    }
    let origin = match request.headers().get(header::ORIGIN) {
        None => None,
        Some(value) => match value.to_str() {
            Ok(text) if config.allowed_origins.iter().any(|o| o == text) => Some(value.clone()),
            _ => {
                return ApiError::new(
                    StatusCode::FORBIDDEN,
                    "origin-not-allowed",
                    "requests from this origin are not allowed",
                )
                .into_response();
            }
        },
    };
    let Some(origin) = origin else {
        return next.run(request).await;
    };
    let preflight = request.method() == Method::OPTIONS
        && request
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
    let mut response = if preflight {
        let mut response = StatusCode::NO_CONTENT.into_response();
        let headers = response.headers_mut();
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, DELETE"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("authorization, content-type"),
        );
        headers.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("600"),
        );
        response
    } else {
        next.run(request).await
    };
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static(EXPOSED),
    );
    headers.append(header::VARY, HeaderValue::from_static("Origin"));
    response
}

/// The scope a method needs: `read` for reads, `delete` for deletes, and
/// `write` for everything else.
pub fn scope_for(method: &Method) -> &'static str {
    match *method {
        Method::GET | Method::HEAD => "read",
        Method::DELETE => "delete",
        _ => "write",
    }
}

/// Every scope; new keys get all of them unless told otherwise.
pub const SCOPES: &[&str] = &["read", "write", "delete"];

/// The usage-record name of a route, such as `diff.sync`.
fn operation(method: &Method, route: Option<&str>) -> &'static str {
    match (method.as_str(), route.unwrap_or("")) {
        ("POST", "/v1/diff") => "diff.sync",
        ("POST", "/v1/blobs") => "blobs.put",
        ("POST", "/v1/jobs") => "jobs.create",
        ("GET", "/v1/jobs/{id}") => "jobs.get",
        ("POST", "/v1/jobs/{id}/cancel") => "jobs.cancel",
        ("GET", "/v1/diffs/{id}") => "diffs.get",
        ("DELETE", "/v1/diffs/{id}") => "diffs.delete",
        ("GET", "/v1/formats") => "formats.get",
        _ => "other",
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token)
}

fn unauthorized(message: &str) -> ApiError {
    ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
}

async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<ApiKey, ApiError> {
    let token =
        bearer(headers).ok_or_else(|| unauthorized("send Authorization: Bearer <API key>"))?;
    api_keys::authenticate(state.db.pool(), token)
        .await?
        .ok_or_else(|| unauthorized("the API key is not valid or has been revoked"))
}

/// Authenticates `/v1` requests, checks the key's scope and rate limit, and
/// appends a usage record.
pub async fn auth(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let started = Instant::now();
    let caller = match state.config.auth_mode {
        AuthMode::None => Caller { key: None },
        AuthMode::ApiKey => match authenticate(&state, request.headers()).await {
            Ok(key) => Caller { key: Some(key) },
            Err(error) => return error.into_response(),
        },
    };
    if let Some(key) = &caller.key {
        let scope = scope_for(request.method());
        if !key.has_scope(scope) {
            return ApiError::new(
                StatusCode::FORBIDDEN,
                "forbidden",
                format!("this API key lacks the `{scope}` scope"),
            )
            .into_response();
        }
        if let Some(limiter) = &state.limiter
            && let Err(not_until) = limiter.check_key(&key.id)
        {
            use governor::clock::{Clock, DefaultClock};
            let wait = not_until.wait_time_from(DefaultClock::default().now());
            let mut error = ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate-limited",
                "too many requests for this API key",
            );
            error.retry_after = Some(wait.as_secs().max(1));
            return error.into_response();
        }
    }
    let operation = operation(
        request.method(),
        request
            .extensions()
            .get::<MatchedPath>()
            .map(MatchedPath::as_str),
    );
    let bytes_in = content_length(request.headers());
    request.extensions_mut().insert(caller.clone());
    let response = next.run(request).await;
    let outcome = response
        .extensions()
        .get::<ErrorCode>()
        .map_or("ok", |c| c.0);
    let usage = Usage {
        api_key_id: caller.key_id(),
        operation,
        outcome,
        bytes_in: i64::try_from(bytes_in).unwrap_or(i64::MAX),
        bytes_out: i64::try_from(body_len(&response)).unwrap_or(i64::MAX),
        duration_ms: i64::try_from(millis(started)).unwrap_or(i64::MAX),
    };
    let pool = state.db.pool().clone();
    tokio::spawn(async move {
        if let Err(error) = records::record_usage(&pool, &usage).await {
            tracing::warn!(code = error.code(), "usage record failed");
        }
    });
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_tokens_are_parsed_strictly() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer(&headers), None);
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc"),
        );
        assert_eq!(bearer(&headers), Some("abc"));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer abc"),
        );
        assert_eq!(bearer(&headers), Some("abc"));
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert_eq!(bearer(&headers), None);
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearerabc"));
        assert_eq!(bearer(&headers), None);
    }

    #[test]
    fn methods_map_to_scopes_and_routes_to_operations() {
        assert_eq!(scope_for(&Method::GET), "read");
        assert_eq!(scope_for(&Method::POST), "write");
        assert_eq!(scope_for(&Method::DELETE), "delete");
        assert_eq!(operation(&Method::POST, Some("/v1/diff")), "diff.sync");
        assert_eq!(
            operation(&Method::DELETE, Some("/v1/diffs/{id}")),
            "diffs.delete"
        );
        assert_eq!(operation(&Method::GET, None), "other");
    }

    #[test]
    fn host_comes_from_the_header_or_an_agreeing_authority() {
        let req = |uri: &str, host: Option<&str>| {
            let mut builder = Request::builder().uri(uri);
            if let Some(host) = host {
                builder = builder.header(header::HOST, host);
            }
            builder.body(axum::body::Body::empty()).unwrap()
        };
        assert_eq!(
            request_host(&req("/x", Some("LocalHost:8080"))),
            Some("localhost:8080".to_owned())
        );
        assert_eq!(request_host(&req("/x", None)), None);
        assert_eq!(
            request_host(&req("http://127.0.0.1:8080/x", None)),
            Some("127.0.0.1:8080".to_owned())
        );
        assert_eq!(
            request_host(&req("http://127.0.0.1:8080/x", Some("evil.example:8080"))),
            None
        );
    }
}
