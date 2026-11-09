//! Requests refused before routing (`docs/api/README.md`, Errors): a request
//! target over 65,534 bytes, bytes past an understated `Content-Length`
//! (read as the next request), a malformed header and a head with too many
//! headers each get the typed error body with a documented code, never
//! hyper's empty answer; and a request with two `Host` headers gets `400`.
//! Runs the real listener on a random loopback port; no database is needed.

mod support;

use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

struct Running {
    addr: String,
    stop: oneshot::Sender<()>,
    task: JoinHandle<Result<(), diffyman_server::serve::ServeError>>,
    _server: support::Server,
}

impl Running {
    async fn start() -> Running {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = support::offline(&[("DIFFYMAN_ALLOWED_HOSTS", addr.as_str())]);
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(diffyman_server::serve::run(
            listener,
            server.state.clone(),
            async {
                let _ = stopped.await;
            },
        ));
        Running {
            addr,
            stop,
            task,
            _server: server,
        }
    }

    async fn stop(self) {
        self.stop.send(()).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(15), self.task).await;
        assert!(matches!(result, Ok(Ok(Ok(())))), "{result:?}");
    }

    /// Sends `data` and reads until the server closes the connection.
    async fn exchange(&self, data: &[u8]) -> Vec<Answer> {
        let mut stream = TcpStream::connect(&self.addr).await.unwrap();
        stream.write_all(data).await.unwrap();
        let mut out = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut out));
        // A reset after the answer is fine; what was read counts.
        let _ = read.await.expect("the server closed the connection");
        answers(&out)
    }
}

#[derive(Debug)]
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// The error code, after checking the typed error shape.
    fn code(&self) -> String {
        assert_eq!(
            self.header("content-type"),
            Some("application/json"),
            "{self:?}"
        );
        assert!(self.header("x-request-id").is_some(), "{self:?}");
        let value: Value =
            serde_json::from_slice(&self.body).unwrap_or_else(|_| panic!("not JSON: {self:?}"));
        let error = value["error"].as_object().expect("an error object");
        assert_eq!(value.as_object().unwrap().len(), 1, "{value}");
        assert_eq!(error.len(), 2, "{value}");
        assert!(error["message"].is_string(), "{value}");
        error["code"].as_str().unwrap().to_owned()
    }
}

/// Every response in `data`, each framed by its `Content-Length`.
fn answers(mut data: &[u8]) -> Vec<Answer> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let end = data
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or_else(|| panic!("a partial head: {:?}", String::from_utf8_lossy(data)));
        let head = String::from_utf8(data[..end].to_vec()).unwrap();
        let mut lines = head.split("\r\n");
        let status = lines.next().unwrap()[9..12].parse().unwrap();
        let headers: Vec<(String, String)> = lines
            .map(|l| {
                let (n, v) = l.split_once(':').unwrap();
                (n.to_ascii_lowercase(), v.trim().to_owned())
            })
            .collect();
        let length: usize = headers
            .iter()
            .find(|(n, _)| n == "content-length")
            .map_or(0, |(_, v)| v.parse().unwrap());
        let body = data[end + 4..end + 4 + length].to_vec();
        data = &data[end + 4 + length..];
        out.push(Answer {
            status,
            headers,
            body,
        });
    }
    out
}

#[tokio::test]
async fn test_request_target_of_64_kib_gets_a_typed_400() {
    let server = Running::start().await;
    for length in [65_536, 70_000] {
        let target = format!("/v1/jobs/{}", "0".repeat(length));
        let request = format!("GET {target} HTTP/1.1\r\nHost: {}\r\n\r\n", server.addr);
        let answers = server.exchange(request.as_bytes()).await;
        assert_eq!(answers.len(), 1, "{answers:?}");
        assert_eq!(answers[0].status, 400, "{answers:?}");
        assert_eq!(answers[0].code(), "invalid-request");
        assert_eq!(answers[0].header("connection"), Some("close"));
    }
    // Just under the limit, it is routed like any other target.
    let target = format!("/v1/jobs/{}", "0".repeat(65_000));
    let request = format!(
        "GET {target} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        server.addr
    );
    let answers = server.exchange(request.as_bytes()).await;
    assert_eq!(answers.len(), 1, "{answers:?}");
    assert_eq!(
        (answers[0].status, answers[0].code().as_str()),
        (404, "job-not-found")
    );
    server.stop().await;
}

#[tokio::test]
async fn test_over_long_sha256_value_gets_invalid_blob_id() {
    let server = Running::start().await;
    let long = format!(
        "POST /v1/blobs?sha256={} HTTP/1.1\r\nHost: {}\r\nContent-Length: 1\r\n\r\nx",
        "a".repeat(100_000),
        server.addr
    );
    let alone = server.exchange(long.as_bytes()).await;
    assert_eq!(alone.len(), 1, "{alone:?}");
    assert_eq!(
        (alone[0].status, alone[0].code().as_str()),
        (400, "invalid-blob-id")
    );
    // The same on a kept-alive connection, after another request.
    let mut stream = TcpStream::connect(&server.addr).await.unwrap();
    let first = format!("GET /nothing HTTP/1.1\r\nHost: {}\r\n\r\n", server.addr);
    stream.write_all(first.as_bytes()).await.unwrap();
    let mut seen = Vec::new();
    let mut buf = [0u8; 4096];
    while answers_in(&seen) < 1 {
        let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(n > 0, "closed after the first request");
        seen.extend_from_slice(&buf[..n]);
    }
    stream.write_all(long.as_bytes()).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut seen)).await;
    let kept = answers(&seen);
    assert_eq!(kept.len(), 2, "{kept:?}");
    assert_eq!(kept[0].code(), "not-found");
    assert_eq!(
        (kept[1].status, kept[1].code().as_str()),
        (400, "invalid-blob-id")
    );
    server.stop().await;
}

/// How many whole responses `data` holds.
fn answers_in(data: &[u8]) -> usize {
    let text = String::from_utf8_lossy(data);
    let Some(end) = text.find("\r\n\r\n") else {
        return 0;
    };
    let length: usize = text[..end]
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .map_or(0, |v| v.trim().parse().unwrap());
    usize::from(data.len() >= end + 4 + length)
}

#[tokio::test]
async fn test_bytes_past_an_understated_content_length_get_a_typed_400() {
    let server = Running::start().await;
    let body = br#"{"left": {"text": "a\n"}, "right": {"text": "b\n"}}"#;
    let declared = body.len() / 2;
    let mut request = format!(
        "POST /v1/diff HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {declared}\r\n\r\n",
        server.addr
    )
    .into_bytes();
    request.extend_from_slice(body);
    let answers = server.exchange(&request).await;
    assert_eq!(answers.len(), 2, "{answers:?}");
    // The declared bytes: a truncated JSON body.
    assert_eq!(
        (answers[0].status, answers[0].code().as_str()),
        (400, "invalid-request")
    );
    // The rest, read as the next request: refused, typed, never a 2xx.
    assert_eq!(
        (answers[1].status, answers[1].code().as_str()),
        (400, "invalid-request")
    );
    let text = String::from_utf8_lossy(&answers[1].body).into_owned();
    assert!(
        !text.contains("right"),
        "the answer quotes the request: {text}"
    );
    server.stop().await;
}

#[tokio::test]
async fn test_malformed_and_oversized_heads_get_typed_errors() {
    let server = Running::start().await;
    let host = &server.addr;
    let cases = [
        (
            format!("GET /healthz HTTP/1.1\r\nHost: {host}\r\nBad Header\r\n\r\n"),
            400,
            "invalid-request",
        ),
        (
            format!(
                "GET /healthz HTTP/1.1\r\nHost: {host}\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n"
            ),
            400,
            "invalid-request",
        ),
        (
            "\x01\x02 / HTTP/1.1\r\n\r\n".to_owned(),
            400,
            "invalid-request",
        ),
        (
            format!(
                "GET /healthz HTTP/1.1\r\nHost: {host}\r\n{}\r\n",
                "X-Many: v\r\n".repeat(120)
            ),
            431,
            "headers-too-large",
        ),
    ];
    for (request, status, code) in cases {
        let answers = server.exchange(request.as_bytes()).await;
        assert_eq!(answers.len(), 1, "{request:?}: {answers:?}");
        assert_eq!(
            (answers[0].status, answers[0].code().as_str()),
            (status, code),
            "{request:?}"
        );
    }
    server.stop().await;
}

#[tokio::test]
async fn test_two_host_headers_get_a_typed_400() {
    let server = Running::start().await;
    let host = &server.addr;
    for (first, second) in [
        (host.as_str(), "evil.example"),
        ("evil.example", host.as_str()),
        (host.as_str(), host.as_str()),
    ] {
        let request = format!(
            "GET /healthz HTTP/1.1\r\nHost: {first}\r\nHost: {second}\r\nConnection: close\r\n\r\n"
        );
        let answers = server.exchange(request.as_bytes()).await;
        assert_eq!(answers.len(), 1, "{answers:?}");
        assert_eq!(
            (answers[0].status, answers[0].code().as_str()),
            (400, "invalid-request"),
            "Host {first}, Host {second}"
        );
    }
    // One allowed Host is still served.
    let request = format!("GET /healthz HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    let answers = server.exchange(request.as_bytes()).await;
    assert_ne!(answers[0].status, 400, "{answers:?}");
    server.stop().await;
}
