//! Connection time limits (`docs/api/README.md`, Limits): a request head
//! must arrive within the header-read limit, and a kept-alive connection is
//! closed after the idle limit, not after the shorter header-read one, and
//! a refused request does not close it. Runs the real listener on a random
//! loopback port; no database is needed.

mod support;

use std::time::{Duration, Instant};

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
    async fn start(header_read_secs: &str, idle_secs: &str) -> Running {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = support::offline(&[
            ("DIFFYMAN_ALLOWED_HOSTS", addr.as_str()),
            ("DIFFYMAN_HEADER_READ_TIMEOUT_SECS", header_read_secs),
            ("DIFFYMAN_IDLE_TIMEOUT_SECS", idle_secs),
        ]);
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

    fn request(&self) -> String {
        format!("GET /nothing HTTP/1.1\r\nHost: {}\r\n\r\n", self.addr)
    }
}

/// Waits until the server closes `stream` (end of stream or an error),
/// discarding what it sends, and returns how long that took.
async fn time_until_closed(
    stream: &mut (impl tokio::io::AsyncRead + Unpin),
    limit: Duration,
) -> Duration {
    let started = Instant::now();
    let mut buf = [0u8; 1024];
    loop {
        let read = tokio::time::timeout(limit, stream.read(&mut buf))
            .await
            .unwrap_or_else(|_| panic!("the connection was still open after {limit:?}"));
        if !matches!(read, Ok(n) if n > 0) {
            return started.elapsed();
        }
    }
}

/// Reads one response (head and `Content-Length` body) and returns its
/// status line.
async fn read_response(stream: &mut TcpStream) -> String {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
            .await
            .expect("a response")
            .unwrap();
        assert!(n > 0, "closed before a whole response");
        data.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&data).into_owned();
        let Some(end) = text.find("\r\n\r\n") else {
            continue;
        };
        let length = text[..end]
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_owned)
            })
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if data.len() >= end + 4 + length {
            return text.lines().next().unwrap_or("").to_owned();
        }
    }
}

#[tokio::test]
async fn test_slow_request_headers_close_the_connection() {
    let server = Running::start("1", "30").await;
    let stream = TcpStream::connect(&server.addr).await.unwrap();
    let (mut reader, mut writer) = stream.into_split();
    writer
        .write_all(b"GET /nothing HTTP/1.1\r\n")
        .await
        .unwrap();
    // One header byte every 200 ms: bytes keep coming, but never a whole head.
    let trickle = tokio::spawn(async move {
        for byte in b"X-Slow: abcdefghijklmnopqrstuvwxyz".iter().cycle() {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if writer.write_all(&[*byte]).await.is_err() {
                break;
            }
        }
    });
    let waited = time_until_closed(&mut reader, Duration::from_secs(5)).await;
    assert!(waited >= Duration::from_millis(800), "{waited:?}");
    trickle.abort();
    server.stop().await;
}

#[tokio::test]
async fn test_silent_new_connection_is_closed_after_the_header_read_limit() {
    let server = Running::start("1", "30").await;
    let mut stream = TcpStream::connect(&server.addr).await.unwrap();
    let waited = time_until_closed(&mut stream, Duration::from_secs(5)).await;
    assert!(waited >= Duration::from_millis(800), "{waited:?}");
    server.stop().await;
}

#[tokio::test]
async fn test_idle_keep_alive_connection_is_closed_after_the_idle_limit() {
    let server = Running::start("1", "3").await;
    let mut stream = TcpStream::connect(&server.addr).await.unwrap();
    stream.write_all(server.request().as_bytes()).await.unwrap();
    assert!(read_response(&mut stream).await.starts_with("HTTP/1.1 404"));

    // Silent for longer than the header-read limit but within the idle one:
    // the connection is still usable.
    tokio::time::sleep(Duration::from_millis(1600)).await;
    stream.write_all(server.request().as_bytes()).await.unwrap();
    assert!(read_response(&mut stream).await.starts_with("HTTP/1.1 404"));

    // Then closed once the idle limit passes.
    let waited = time_until_closed(&mut stream, Duration::from_secs(8)).await;
    assert!(waited >= Duration::from_millis(2500), "{waited:?}");
    server.stop().await;
}

/// A request answered without its body being used (a refused method or
/// `Host`) leaves a kept-alive connection usable even when the body arrives
/// after the head, as it does from clients that write them separately.
/// Before small bodies were read ahead, the server closed such a connection
/// after the answer, without `Connection: close`, and the client's next
/// request on it (in CI, a 200 MB upload) failed with a broken pipe.
#[tokio::test]
async fn test_refused_request_with_a_late_body_keeps_the_connection() {
    let server = Running::start("10", "30").await;
    let body = br#"{"left":{"text":"a\n"},"right":{"text":"b\n"}}"#;
    for (head, status) in [
        (
            format!("PUT /v1/diff HTTP/1.1\r\nHost: {}\r\n", server.addr),
            "405",
        ),
        (
            "POST /v1/diff HTTP/1.1\r\nHost: evil.example\r\n".to_owned(),
            "403",
        ),
    ] {
        let mut stream = TcpStream::connect(&server.addr).await.unwrap();
        let head = format!(
            "{head}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        stream.write_all(body).await.unwrap();
        let answer = read_response(&mut stream).await;
        assert!(
            answer.starts_with(&format!("HTTP/1.1 {status}")),
            "{answer}"
        );
        // The same connection serves the next request.
        stream.write_all(server.request().as_bytes()).await.unwrap();
        let next = read_response(&mut stream).await;
        assert!(next.starts_with("HTTP/1.1 404"), "after {status}: {next}");
    }
    server.stop().await;
}

/// A large body is not read ahead: a refusal is answered without waiting
/// for it.
#[tokio::test]
async fn test_refused_request_with_a_large_body_is_answered_at_once() {
    let server = Running::start("10", "30").await;
    let mut stream = TcpStream::connect(&server.addr).await.unwrap();
    let head = "POST /v1/blobs HTTP/1.1\r\nHost: evil.example\r\n\
                Content-Type: application/octet-stream\r\nContent-Length: 10000000\r\n\r\n";
    stream.write_all(head.as_bytes()).await.unwrap();
    let started = Instant::now();
    let answer = read_response(&mut stream).await;
    assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    server.stop().await;
}
