//! Typed answers for requests hyper refuses before routing.
//!
//! hyper answers a request head it cannot parse itself, before any service
//! or middleware sees a request, with a status and an empty body: `400` for
//! a malformed request line or header (including the bytes after a body
//! whose `Content-Length` understated it, read as the next request), `414`
//! for a request target over 65,534 bytes and `431` for a head over its
//! buffer (about 400 KiB) or with more than 100 headers. It has no hook to
//! change that answer, so [`crate::conn::TimedIo`] recognises it on the way
//! out and writes [`typed`]'s answer in its place: the same typed error body
//! as every other refusal, `X-Request-Id`, and one log line. The code is
//! `invalid-request` (`headers-too-large` for `431`), or `invalid-blob-id`
//! when the start of the refused request, as the connection kept it, shows
//! a `?sha256=` value of `POST /v1/blobs` that is not a blob ID.
//!
//! Only hyper's own answer matches: a whole response head with a status of
//! 400, 414 or 431 and no header but `content-length: 0`, `date` and
//! `connection`. Every response of the router carries `content-type` and
//! `x-request-id`, and no JSON body holds a raw line break, so nothing the
//! server itself sends is rewritten.
//!
//! Which transport-level refusals still carry no body, and why: a connection
//! closed by the header-read or idle limit ([`crate::conn`]) or by the
//! client gets no answer, since no request was received to answer (and the
//! client may be sending nothing at all); an HTTP/2 connection preface is
//! closed by hyper without one, since it is not HTTP/1.1.

use axum::http::StatusCode;
use uuid::Uuid;

use crate::error::{ErrorBody, ErrorDetail};

/// Longer than any head hyper writes for a parse error (status line,
/// `content-length`, `date` and `connection`).
const MAX_HEAD_BYTES: usize = 256;

/// The typed answer that replaces `head` when `head` is hyper's own answer
/// to a request it could not parse, else `None`. Logs the refusal.
///
/// `request_start` gives the first bytes of the refused request, when the
/// connection knows them: a target over the limit whose length comes from
/// the `?sha256=` value of `POST /v1/blobs` is that route's
/// `invalid-blob-id`, as a shorter malformed value is.
pub fn typed(head: &[u8], request_start: impl FnOnce() -> Vec<u8>) -> Option<Vec<u8>> {
    let (status, date) = hyper_rejection(head)?;
    let (status, code, message) = match status {
        414 if bad_blob_id(&request_start()) => (
            StatusCode::BAD_REQUEST,
            "invalid-blob-id",
            "invalid blob id: expected 64 lowercase hex digits",
        ),
        414 => (
            StatusCode::BAD_REQUEST,
            "invalid-request",
            "the request target is longer than 65534 bytes",
        ),
        431 => (
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            "headers-too-large",
            "the request head is larger than the server reads, or has more than 100 headers",
        ),
        _ => (
            StatusCode::BAD_REQUEST,
            "invalid-request",
            "the request is not valid HTTP/1.1",
        ),
    };
    let body = serde_json::to_vec(&ErrorBody {
        error: ErrorDetail {
            code: code.to_owned(),
            message: message.to_owned(),
        },
    })
    .ok()?;
    let request_id = Uuid::new_v4().simple().to_string()[..16].to_owned();
    let mut out = format!(
        "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\nx-request-id: {request_id}\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or(""),
        body.len()
    );
    if let Some(date) = date {
        out.push_str("date: ");
        out.push_str(date);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    let mut out = out.into_bytes();
    out.extend_from_slice(&body);
    tracing::info!(
        request_id = request_id.as_str(),
        route = "unparsed",
        status = status.as_u16(),
        code,
        bytes_out = body.len(),
        "request"
    );
    Some(out)
}

/// Whether a request starting with `start` is `POST /v1/blobs` whose first
/// `sha256` query value is not a blob ID: it runs past the end of `start`,
/// or it is not 64 lowercase hex digits (a percent-encoded value is not
/// judged).
fn bad_blob_id(start: &[u8]) -> bool {
    let start = start.trim_ascii_start();
    let Some(query) = start.strip_prefix(b"POST /v1/blobs?") else {
        return false;
    };
    let mut rest = query;
    loop {
        let field_end = rest
            .iter()
            .position(|b| matches!(b, b'&' | b' ' | b'\r' | b'\n'));
        let field = &rest[..field_end.unwrap_or(rest.len())];
        if let Some(value) = field.strip_prefix(b"sha256=") {
            // A value cut off by the end of `start` is longer than any ID.
            let cut = field_end.is_none();
            let id =
                value.len() == 64 && value.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
            return cut || (!id && !value.contains(&b'%'));
        }
        match field_end {
            Some(end) if rest[end] == b'&' => rest = &rest[end + 1..],
            _ => return false,
        }
    }
}

/// The status and `date` value of `head` when it is a whole response head
/// as hyper writes it for a parse error: `HTTP/1.1 400`, `414` or `431`,
/// then only `content-length: 0`, `date` and `connection` headers, then the
/// blank line and nothing after it.
fn hyper_rejection(head: &[u8]) -> Option<(u16, Option<&str>)> {
    // Every write passes here: refuse anything else at once.
    if head.len() > MAX_HEAD_BYTES || !head.starts_with(b"HTTP/1.1 4") {
        return None;
    }
    let text = std::str::from_utf8(head).ok()?;
    let text = text.strip_prefix("HTTP/1.1 ")?.strip_suffix("\r\n\r\n")?;
    let mut lines = text.split("\r\n");
    let status_line = lines.next()?;
    let status = match status_line.get(..3)? {
        "400" => 400,
        "414" => 414,
        "431" => 431,
        _ => return None,
    };
    if status_line.as_bytes().get(3).is_some_and(|b| *b != b' ') {
        return None;
    }
    let (mut length, mut date) = (false, None);
    for line in lines {
        let (name, value) = line.split_once(": ")?;
        match name.to_ascii_lowercase().as_str() {
            "content-length" if value == "0" => length = true,
            "date" if date.is_none() => date = Some(value),
            "connection" => {}
            _ => return None,
        }
    }
    length.then_some((status, date))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(head: &str) -> Option<(String, serde_json::Value)> {
        let out = String::from_utf8(typed(head.as_bytes(), Vec::new)?).unwrap();
        let (head, body) = out.split_once("\r\n\r\n").unwrap();
        Some((head.to_owned(), serde_json::from_str(body).unwrap()))
    }

    #[test]
    fn hyper_parse_error_answers_become_typed_errors() {
        let date = "date: Sun, 08 Nov 2026 10:00:00 GMT";
        for (hyper, status, code) in [
            ("HTTP/1.1 400 Bad Request", "400", "invalid-request"),
            ("HTTP/1.1 414 URI Too Long", "400", "invalid-request"),
            (
                "HTTP/1.1 431 Request Header Fields Too Large",
                "431",
                "headers-too-large",
            ),
        ] {
            let (head, body) = answer(&format!("{hyper}\r\ncontent-length: 0\r\n{date}\r\n\r\n"))
                .unwrap_or_else(|| panic!("{hyper} not replaced"));
            assert!(head.starts_with(&format!("HTTP/1.1 {status} ")), "{head}");
            assert!(head.contains("\r\ncontent-type: application/json\r\n"));
            assert!(head.contains("\r\nconnection: close\r\n"));
            assert!(head.contains("\r\nx-request-id: "));
            assert!(head.contains(&format!("\r\n{date}")));
            assert_eq!(body["error"]["code"], code);
            assert_eq!(body.as_object().unwrap().len(), 1);
            let codes = crate::error::CODES;
            assert!(
                codes
                    .iter()
                    .any(|(c, s)| *c == code && s.to_string() == status)
            );
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .unwrap();
            assert_eq!(length, body.to_string().len().to_string());
        }
        // With `connection: close` and without a date.
        assert!(
            answer("HTTP/1.1 400 Bad Request\r\nconnection: close\r\ncontent-length: 0\r\n\r\n")
                .is_some()
        );
    }

    #[test]
    fn other_responses_are_left_alone() {
        for head in [
            // The router's own answers carry content-type and x-request-id.
            "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: 0\r\n\r\n",
            "HTTP/1.1 400 Bad Request\r\nx-request-id: 0123456789abcdef\r\ncontent-length: 0\r\n\r\n",
            // Other statuses, a body, no length, a partial head, not a head.
            "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n",
            "HTTP/1.1 100 Continue\r\n\r\n",
            "HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n{}",
            "HTTP/1.1 400 Bad Request\r\ncontent-length: 2\r\n\r\n",
            "HTTP/1.1 400 Bad Request\r\n\r\n",
            "HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n",
            "HTTP/1.1 4000\r\ncontent-length: 0\r\n\r\n",
            "{\"error\":1}",
            "",
        ] {
            assert!(typed(head.as_bytes(), Vec::new).is_none(), "{head:?}");
        }
    }

    #[test]
    fn an_over_long_sha256_value_is_an_invalid_blob_id() {
        let hyper = b"HTTP/1.1 414 URI Too Long\r\ncontent-length: 0\r\n\r\n";
        let code = |start: &str| {
            let out = typed(hyper, || start.as_bytes().to_vec()).unwrap();
            let out = String::from_utf8(out).unwrap();
            let body: serde_json::Value =
                serde_json::from_str(out.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert!(out.starts_with("HTTP/1.1 400 "), "{out}");
            body["error"]["code"].as_str().unwrap().to_owned()
        };
        let hex = "a".repeat(64);
        let long = "a".repeat(200);
        for start in [
            format!("POST /v1/blobs?sha256={long}"),
            format!("\r\nPOST /v1/blobs?x=1&sha256={long}"),
            "POST /v1/blobs?sha256=ABC&pad=xxxx".to_owned(),
            format!("POST /v1/blobs?sha256={hex}0&pad=xxxx"),
        ] {
            assert_eq!(code(&start), "invalid-blob-id", "{start}");
        }
        for start in [
            String::new(),
            format!("POST /v1/blobs?sha256={hex}&pad={long}"),
            format!("POST /v1/blobs?pad={long}"),
            // Percent-encoded: may decode to an ID, so not judged here.
            format!("POST /v1/blobs?sha256=%61{}&pad={long}", &hex[1..]),
            format!("POST /v1/diff?sha256={long}"),
            format!("GET /v1/blobs?sha256={long}"),
            format!("GET /v1/jobs/{long}"),
        ] {
            assert_eq!(code(&start), "invalid-request", "{start}");
        }
    }
}
