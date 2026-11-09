//! HTTP-level tests of the router, the worker and the sweeper.
//!
//! Tests that need no database run against a pool that never connects (its
//! queries fail with `db-unavailable`), with authentication off unless the
//! test checks authentication. Tests that need one run each in their own
//! schema of the `diffyman_test` database (the harness of `diffyman-db`) and
//! skip, printing why, when no database is reachable; set
//! `DIFFYMAN_TEST_DB_REQUIRED=1` to make that a failure.

#[allow(dead_code)]
#[path = "../../diffyman-db/tests/common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use base64::Engine as _;
use bytes::Bytes;
use diffyman::model::DiffDocument;
use diffyman_db::jobs::{self, JobStatus};
use diffyman_db::{Db, blobs};
use diffyman_server::config::{Config, Sources};
use diffyman_server::{AppState, Tuning, keys, sweeper, worker};
use diffyman_storage::{BlobId, BlobStore, StoreOptions};
use serde_json::{Value, json};
use tower::ServiceExt;

const HOST: &str = "127.0.0.1:8080";

macro_rules! test_db {
    ($name:literal) => {
        match common::test_db($name).await {
            Some(t) => t,
            None => return,
        }
    };
}

/// A configuration for tests: memory store, everything under a temporary
/// home folder, and `vars` on top.
fn config(home: &Path, vars: &[(&str, &str)]) -> Config {
    let mut all: BTreeMap<String, String> = [
        ("DIFFYMAN_PG_PASSWORD", "unused"),
        ("DIFFYMAN_STORE_URL", "memory://"),
        ("DIFFYMAN_JOB_LEASE_SECS", "6"),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect();
    for (k, v) in vars {
        all.insert((*k).to_owned(), (*v).to_owned());
    }
    Config::load(Sources {
        vars: all,
        cwd: home.to_owned(),
        home: Some(home.to_owned()),
        cpus: 4,
    })
    .expect("test configuration")
}

fn tuning() -> Tuning {
    Tuning {
        poll_interval: Duration::from_millis(50),
        retry_delay: Duration::from_millis(100),
        shutdown_grace: Duration::from_secs(2),
    }
}

struct Server {
    state: AppState,
    app: Router,
    _home: tempfile::TempDir,
}

fn build(config: Config, db: Db) -> (AppState, Router) {
    let store = diffyman_storage::open(
        &config.store_url,
        StoreOptions {
            max_blob_bytes: config.limits.max_blob_bytes,
            ..StoreOptions::default()
        },
    )
    .unwrap();
    build_with(config, db, store)
}

fn build_with(config: Config, db: Db, store: Arc<dyn BlobStore>) -> (AppState, Router) {
    let state = AppState::new(config, db, store, tuning());
    let app = diffyman_server::router(state.clone());
    (state, app)
}

/// A server whose database is never reachable.
fn offline(vars: &[(&str, &str)]) -> Server {
    let home = tempfile::tempdir().unwrap();
    let mut all = vec![("DIFFYMAN_AUTH_MODE", "none")];
    all.extend_from_slice(vars);
    let config = config(home.path(), &all);
    let options = sqlx::postgres::PgConnectOptions::new()
        .host("127.0.0.1")
        .port(1)
        .username("nobody")
        .database("nothing");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(500))
        .connect_lazy_with(options);
    let (state, app) = build(config, Db::from_pool(pool));
    Server {
        state,
        app,
        _home: home,
    }
}

/// A server on a test database.
fn online(t: &common::TestDb, vars: &[(&str, &str)]) -> Server {
    let home = tempfile::tempdir().unwrap();
    let (state, app) = build(config(home.path(), vars), t.db.clone());
    Server {
        state,
        app,
        _home: home,
    }
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| {
            panic!(
                "not JSON ({}): {}",
                self.status,
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    fn code(&self) -> String {
        self.json()["error"]["code"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }

    fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }
}

fn request(method: Method, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, HOST)
}

async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Reply {
        status,
        headers,
        body,
    }
}

async fn get(app: &Router, uri: &str, key: Option<&str>) -> Reply {
    let mut builder = request(Method::GET, uri);
    if let Some(key) = key {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    send(app, builder.body(Body::empty()).unwrap()).await
}

async fn post_json(app: &Router, uri: &str, key: Option<&str>, body: &Value) -> Reply {
    let mut builder = request(Method::POST, uri).header(header::CONTENT_TYPE, "application/json");
    if let Some(key) = key {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    send(app, builder.body(Body::from(body.to_string())).unwrap()).await
}

async fn post_bytes(app: &Router, uri: &str, key: Option<&str>, bytes: &[u8]) -> Reply {
    let mut builder = request(Method::POST, uri)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.len());
    if let Some(key) = key {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    send(app, builder.body(Body::from(bytes.to_vec())).unwrap()).await
}

fn text_diff(left: &str, right: &str) -> Value {
    json!({
        "left": {"name": "a.txt", "text": left},
        "right": {"name": "b.txt", "text": right},
        "options": {"context": 1},
    })
}

const BOUNDARY: &str = "diffyman-test-boundary";

fn multipart(parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (field, file, data) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        match file {
            Some(file) => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{field}\"; filename=\"{file}\"\r\n\
                     Content-Type: application/octet-stream\r\n\r\n"
                )
                .as_bytes(),
            ),
            None => body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{field}\"\r\n\r\n").as_bytes(),
            ),
        }
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

async fn post_multipart(app: &Router, uri: &str, body: Vec<u8>) -> Reply {
    let request = request(Method::POST, uri)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(body))
        .unwrap();
    send(app, request).await
}

fn conformance(json: &[u8]) -> String {
    let doc: DiffDocument = serde_json::from_slice(json).unwrap();
    diffyman::model::to_conformance_json(&doc).unwrap()
}

// ---------------------------------------------------------------------------
// Without a database.

#[tokio::test]
async fn wrong_host_and_browser_origins_are_refused() {
    let server = offline(&[]);
    let ok = get(&server.app, "/openapi.json", None).await;
    assert_eq!(ok.status, StatusCode::OK);
    assert!(!ok.header("x-request-id").is_empty());

    for host in ["evil.example:8080", "127.0.0.1:9999", "localhost"] {
        let reply = send(
            &server.app,
            Request::builder()
                .uri("/openapi.json")
                .header(header::HOST, host)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{host}");
        assert_eq!(reply.code(), "host-not-allowed");
    }
    let no_host = send(
        &server.app,
        Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(no_host.code(), "host-not-allowed");
    let upper = send(
        &server.app,
        Request::builder()
            .uri("/openapi.json")
            .header(header::HOST, "LOCALHOST:8080")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(upper.status, StatusCode::OK, "hosts compare without case");

    for origin in ["http://evil.example", "null", "http://localhost:5173"] {
        let reply = send(
            &server.app,
            request(Method::POST, "/v1/diff")
                .header(header::ORIGIN, origin)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(text_diff("a\n", "b\n").to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{origin}");
        assert_eq!(reply.code(), "origin-not-allowed");
        assert_eq!(reply.header("access-control-allow-origin"), "");
    }

    let allowed = offline(&[("DIFFYMAN_ALLOWED_ORIGINS", "http://localhost:5173")]);
    let preflight = send(
        &allowed.app,
        request(Method::OPTIONS, "/v1/diff")
            .header(header::ORIGIN, "http://localhost:5173")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(preflight.status, StatusCode::NO_CONTENT);
    assert_eq!(
        preflight.header("access-control-allow-origin"),
        "http://localhost:5173"
    );
    assert!(
        preflight
            .header("access-control-allow-headers")
            .contains("authorization")
    );
    let reply = send(
        &allowed.app,
        request(Method::POST, "/v1/diff")
            .header(header::ORIGIN, "http://localhost:5173")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(text_diff("a\n", "b\n").to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.header("access-control-allow-origin"),
        "http://localhost:5173"
    );
}

#[tokio::test]
async fn v1_routes_need_an_api_key() {
    let home = tempfile::tempdir().unwrap();
    let server = {
        let config = config(home.path(), &[]);
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(500))
            .connect_lazy_with(
                sqlx::postgres::PgConnectOptions::new()
                    .host("127.0.0.1")
                    .port(1),
            );
        build(config, Db::from_pool(pool)).1
    };
    for (method, uri) in [
        (Method::POST, "/v1/diff"),
        (Method::POST, "/v1/blobs"),
        (Method::POST, "/v1/jobs"),
        (Method::GET, "/v1/jobs/0190f0c4-7b9a-7cc0-8000-000000000000"),
        (
            Method::POST,
            "/v1/jobs/0190f0c4-7b9a-7cc0-8000-000000000000/cancel",
        ),
        (
            Method::GET,
            "/v1/diffs/0190f0c4-7b9a-7cc0-8000-000000000000",
        ),
        (
            Method::DELETE,
            "/v1/diffs/0190f0c4-7b9a-7cc0-8000-000000000000",
        ),
        (Method::GET, "/v1/formats"),
    ] {
        let reply = send(
            &server,
            request(method.clone(), uri).body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method} {uri}");
        assert_eq!(reply.code(), "unauthorized");
        assert_eq!(reply.header("www-authenticate"), "Bearer");
        for bad in ["Bearer nonsense", "Basic dXNlcjpwYXNz", "Bearer dmk_0"] {
            let reply = send(
                &server,
                request(method.clone(), uri)
                    .header(header::AUTHORIZATION, bad)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(
                reply.status,
                StatusCode::UNAUTHORIZED,
                "{method} {uri} {bad}"
            );
        }
    }
    // Health and the API description need no key.
    assert_eq!(
        get(&server, "/openapi.json", None).await.status,
        StatusCode::OK
    );
    let health = get(&server, "/healthz", None).await;
    assert_eq!(health.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        health.json(),
        json!({"status": "unavailable", "checks": {"database": "unavailable", "storage": "ok"}})
    );
}

#[tokio::test]
async fn errors_are_typed_json() {
    let server = offline(&[]);
    let app = &server.app;
    let reply = get(app, "/v1/nothing", None).await;
    assert_eq!(
        (reply.status, reply.code().as_str()),
        (StatusCode::NOT_FOUND, "not-found")
    );
    let reply = get(app, "/v1/diff", None).await;
    assert_eq!(
        (reply.status, reply.code().as_str()),
        (StatusCode::METHOD_NOT_ALLOWED, "method-not-allowed")
    );
    let reply = send(
        app,
        request(Method::POST, "/v1/diff")
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from("x"))
            .unwrap(),
    )
    .await;
    assert_eq!(reply.code(), "unsupported-media-type");
    let reply = send(
        app,
        request(Method::POST, "/v1/diff")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{not json"))
            .unwrap(),
    )
    .await;
    assert_eq!(
        (reply.status, reply.code().as_str()),
        (StatusCode::BAD_REQUEST, "invalid-request")
    );
    let mut body = text_diff("a\n", "b\n");
    body["options"] = json!({"granularity": "sentence"});
    let reply = post_json(app, "/v1/diff", None, &body).await;
    assert_eq!(reply.code(), "invalid-options");
    let reply = post_json(app, "/v1/diff", None, &json!({"left": {"text": "a"}})).await;
    assert_eq!(reply.code(), "invalid-request");
    let reply = post_json(app, "/v1/diff?save=maybe", None, &text_diff("a", "b")).await;
    assert_eq!(reply.code(), "invalid-request");
    let reply = post_bytes(app, "/v1/blobs?sha256=XYZ", None, b"x").await;
    assert_eq!(reply.code(), "invalid-blob-id");
    // The database is down: typed 503 with Retry-After.
    let reply = get(app, "/v1/jobs/0190f0c4-7b9a-7cc0-8000-000000000000", None).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.code(), "db-unavailable");
    assert_eq!(reply.header("retry-after"), "5");
    // An unparsable ID is simply not found.
    let reply = get(app, "/v1/diffs/not-a-uuid", None).await;
    assert_eq!(reply.code(), "diff-not-found");

    let formats = get(app, "/v1/formats", None).await.json();
    assert_eq!(formats["schemaVersion"], diffyman::model::SCHEMA_VERSION);
    assert_eq!(formats["limits"]["maxSyncInputBytes"], 8 << 20);
    assert_eq!(formats["kinds"][2]["kind"], "tree");
}

#[tokio::test]
async fn size_limits() {
    let server = offline(&[
        ("DIFFYMAN_MAX_SYNC_INPUT_BYTES", "16"),
        ("DIFFYMAN_MAX_BODY_BYTES", "1024"),
        ("DIFFYMAN_MAX_BLOB_BYTES", "64"),
    ]);
    let app = &server.app;
    let ok = post_json(app, "/v1/diff", None, &text_diff("0123456789abcdef", "x")).await;
    assert_eq!(ok.status, StatusCode::OK);
    let reply = post_json(app, "/v1/diff", None, &text_diff("0123456789abcdefg", "x")).await;
    assert_eq!(
        (reply.status, reply.code().as_str()),
        (StatusCode::PAYLOAD_TOO_LARGE, "input-too-large")
    );
    let encoded = base64::engine::general_purpose::STANDARD.encode([7u8; 17]);
    let body = json!({"left": {"base64": encoded}, "right": {"text": ""}});
    assert_eq!(
        post_json(app, "/v1/diff", None, &body).await.code(),
        "input-too-large"
    );
    let body = json!({"left": {"text": "a".repeat(2000)}, "right": {"text": ""}});
    let reply = post_json(app, "/v1/diff", None, &body).await;
    assert_eq!(reply.code(), "payload-too-large");

    let form = multipart(&[
        ("left", Some("l.txt"), &[b'a'; 17]),
        ("right", Some("r.txt"), b"b"),
    ]);
    assert_eq!(
        post_multipart(app, "/v1/diff", form).await.code(),
        "input-too-large"
    );
    let form = multipart(&[
        ("left", Some("l.txt"), &[b'a'; 1100]),
        ("right", Some("r.txt"), b"b"),
    ]);
    let reply = post_multipart(app, "/v1/diff", form).await;
    assert_eq!(
        reply.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        reply.code()
    );

    // Blobs: refused by the declared length, and by counting a stream.
    let reply = post_bytes(app, "/v1/blobs", None, &[1u8; 65]).await;
    assert_eq!(
        (reply.status, reply.code().as_str()),
        (StatusCode::PAYLOAD_TOO_LARGE, "blob-too-large")
    );
    let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
        Ok(Bytes::from(vec![1u8; 40])),
        Ok(Bytes::from(vec![2u8; 40])),
    ];
    let streamed = request(Method::POST, "/v1/blobs")
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from_stream(futures::stream::iter(chunks)))
        .unwrap();
    assert_eq!(send(app, streamed).await.code(), "blob-too-large");
}

#[tokio::test]
async fn sync_diffs_are_admitted_before_their_body_is_read() {
    let server = offline(&[
        ("DIFFYMAN_MAX_PARALLEL_DIFFS", "1"),
        ("DIFFYMAN_REQUEST_TIMEOUT_SECS", "2"),
    ]);
    let app = &server.app;
    let held = server
        .state
        .sync
        .admit(1, Duration::from_secs(1))
        .await
        .unwrap();
    // A body that never ends: read before admission, it would hold memory
    // until the request timed out (`timeout`) instead of being refused.
    let endless = request(Method::POST, "/v1/diff")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from_stream(futures::stream::pending::<
            Result<Bytes, std::io::Error>,
        >()))
        .unwrap();
    let reply = send(app, endless).await;
    assert_eq!(
        (reply.status, reply.code().as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "busy")
    );
    drop(held);
    let ok = post_json(app, "/v1/diff", None, &text_diff("a\n", "b\n")).await;
    assert_eq!(ok.status, StatusCode::OK);

    // The memory budget is reserved by the declared body size.
    let server = offline(&[
        ("DIFFYMAN_SYNC_MEMORY_BYTES", "1048576"),
        ("DIFFYMAN_REQUEST_TIMEOUT_SECS", "2"),
    ]);
    let app = &server.app;
    let held = server
        .state
        .sync
        .admit(600 << 10, Duration::from_secs(1))
        .await
        .unwrap();
    let sized = |body: &Value, length: usize| {
        request(Method::POST, "/v1/diff")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, length)
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let small = text_diff("a\n", "b\n");
    let length = small.to_string().len();
    assert_eq!(
        send(app, sized(&small, length)).await.status,
        StatusCode::OK
    );
    let large = json!({"left": {"text": "x".repeat(200 << 10)}, "right": {"text": ""}});
    let length = large.to_string().len();
    assert_eq!(send(app, sized(&large, length)).await.code(), "busy");
    drop(held);
    assert_eq!(
        send(app, sized(&large, length)).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn multipart_requests_match_json_requests() {
    let server = offline(&[]);
    let form = multipart(&[
        ("left", Some("dir/a.txt"), b"one\ntwo\n"),
        ("right", Some("b.txt"), b"one\nthree\n"),
        ("options", None, br#"{"granularity": "word"}"#),
    ]);
    let from_form = post_multipart(&server.app, "/v1/diff", form).await;
    assert_eq!(from_form.status, StatusCode::OK);
    assert_eq!(from_form.header("content-type"), "application/json");
    let body = json!({
        "left": {"name": "a.txt", "text": "one\ntwo\n"},
        "right": {"name": "b.txt", "base64": base64::engine::general_purpose::STANDARD.encode("one\nthree\n")},
        "options": {"granularity": "word"},
    });
    let from_json = post_json(&server.app, "/v1/diff", None, &body).await;
    assert_eq!(conformance(&from_form.body), conformance(&from_json.body));
    let bad = multipart(&[("left", Some("a"), b"x"), ("middle", None, b"y")]);
    assert_eq!(
        post_multipart(&server.app, "/v1/diff", bad).await.code(),
        "invalid-request"
    );
    let missing = multipart(&[("left", Some("a"), b"x")]);
    assert_eq!(
        post_multipart(&server.app, "/v1/diff", missing)
            .await
            .code(),
        "invalid-request"
    );
}

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

/// Every corpus case whose inputs are files: `(case, left, right, options)`.
fn file_cases() -> Vec<(String, PathBuf, PathBuf, String)> {
    let mut cases = Vec::new();
    let mut groups: Vec<_> = std::fs::read_dir(corpus_root())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    groups.sort();
    for group in groups {
        let mut dirs: Vec<_> = std::fs::read_dir(&group)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.join("options.json").is_file())
            .collect();
        dirs.sort();
        for dir in dirs {
            let side = |prefix: &str| {
                std::fs::read_dir(&dir)
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .find(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n == prefix || n.starts_with(&format!("{prefix}.")))
                    })
                    .unwrap()
            };
            let (left, right) = (side("left"), side("right"));
            if left.is_dir() || right.is_dir() {
                continue;
            }
            let options = std::fs::read_to_string(dir.join("options.json")).unwrap();
            let name = dir
                .strip_prefix(corpus_root())
                .unwrap()
                .display()
                .to_string();
            cases.push((name, left, right, options));
        }
    }
    cases
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap().to_str().unwrap().to_owned()
}

#[tokio::test]
async fn sync_diffs_equal_the_engine_on_the_corpus() {
    let server = offline(&[]);
    let cases = file_cases();
    assert!(cases.len() > 150, "found {} cases", cases.len());
    for (i, (case, left_path, right_path, options)) in cases.iter().enumerate() {
        let (left, right) = (
            std::fs::read(left_path).unwrap(),
            std::fs::read(right_path).unwrap(),
        );
        let (left_name, right_name) = (file_name(left_path), file_name(right_path));
        // Alternate the two request shapes.
        let reply = if i % 2 == 0 {
            let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
            let options: Value = serde_json::from_str(options).unwrap();
            let body = json!({
                "left": {"name": left_name, "base64": b64(&left)},
                "right": {"name": right_name, "base64": b64(&right)},
                "options": options,
            });
            post_json(&server.app, "/v1/diff", None, &body).await
        } else {
            let form = multipart(&[
                ("left", Some(&left_name), &left),
                ("right", Some(&right_name), &right),
                ("options", None, options.as_bytes()),
            ]);
            post_multipart(&server.app, "/v1/diff", form).await
        };
        assert_eq!(
            reply.status,
            StatusCode::OK,
            "{case}: {}",
            String::from_utf8_lossy(&reply.body)
        );
        let engine = diffyman::diff_inputs_with(
            diffyman::Input::named(&left, &left_name),
            diffyman::Input::named(&right, &right_name),
            options,
            &diffyman::Settings {
                binding: "cli".to_owned(),
            },
        )
        .unwrap();
        // The body is the engine's canonical JSON, untouched.
        let doc: DiffDocument = serde_json::from_slice(&reply.body).unwrap();
        assert_eq!(
            diffyman::model::to_canonical_json(&doc).unwrap().as_bytes(),
            &reply.body[..],
            "{case}: not canonical"
        );
        assert_eq!(doc.provenance.binding, "server", "{case}");
        assert_eq!(
            conformance(&reply.body),
            conformance(engine.as_bytes()),
            "{case}"
        );
        let expected = std::fs::read(corpus_root().join(case).join("expected.json")).unwrap();
        assert_eq!(
            conformance(&reply.body),
            conformance(&expected),
            "{case}: expected.json"
        );
    }
}

#[tokio::test]
async fn serve_listens_and_shuts_down_gracefully() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let host = addr.to_string();
    let server = offline(&[("DIFFYMAN_ALLOWED_HOSTS", host.as_str())]);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let running = tokio::spawn(diffyman_server::serve::run(
        listener,
        server.state.clone(),
        async {
            let _ = stopped.await;
        },
    ));
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let ask = format!("GET /openapi.json HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(ask.as_bytes()).await.unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.unwrap();
    let answer = String::from_utf8_lossy(&answer);
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.contains("\"openapi\": \"3.1"), "{answer}");
    stop.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(15), running).await;
    assert!(
        matches!(result, Ok(Ok(Ok(())))),
        "did not stop cleanly: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// With a database.

async fn new_key(state: &AppState, scopes: &[&str]) -> (uuid::Uuid, String) {
    let scopes: Vec<String> = scopes.iter().map(|s| (*s).to_owned()).collect();
    let (key, secret) = keys::create(&state.db, "test", &scopes).await.unwrap();
    (key.id, secret.expose().to_owned())
}

#[tokio::test]
async fn api_keys_scopes_revocation_and_rate_limits() {
    let t = test_db!("api_keys_scopes_revocation_and_rate_limits");
    let server = online(&t, &[("DIFFYMAN_RATE_LIMIT_PER_MINUTE", "2")]);
    let app = &server.app;
    let (reader_id, reader) = new_key(&server.state, &["read"]).await;
    assert_eq!(
        get(app, "/v1/formats", Some(&reader)).await.status,
        StatusCode::OK
    );
    let reply = post_json(app, "/v1/diff", Some(&reader), &text_diff("a", "b")).await;
    assert_eq!(
        (reply.status, reply.code().as_str()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    assert_eq!(
        get(app, "/v1/formats", Some(&reader)).await.status,
        StatusCode::OK
    );
    let limited = get(app, "/v1/formats", Some(&reader)).await;
    assert_eq!(
        (limited.status, limited.code().as_str()),
        (StatusCode::TOO_MANY_REQUESTS, "rate-limited")
    );
    assert!(limited.header("retry-after").parse::<u64>().unwrap() >= 1);

    let (_, writer) = new_key(&server.state, &[]).await;
    let reply = post_json(app, "/v1/diff", Some(&writer), &text_diff("a\n", "b\n")).await;
    assert_eq!(reply.status, StatusCode::OK);

    assert!(keys::revoke(&server.state.db, reader_id).await.unwrap());
    let reply = get(app, "/v1/formats", Some(&reader)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(
        keys::create(&server.state.db, "x", &["admin".to_owned()])
            .await
            .is_err()
    );

    // Usage records are appended in the background, for requests that
    // passed the scope and rate checks.
    let mut calls = 0;
    for _ in 0..50 {
        calls = diffyman_db::records::usage_totals(
            t.db.pool(),
            reader_id,
            chrono::Utc::now() - chrono::TimeDelta::hours(1),
        )
        .await
        .unwrap()
        .calls;
        if calls >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(calls, 2, "usage records");
    drop(server);
    t.finish().await;
}

#[tokio::test]
async fn saved_diffs_are_private_and_deletable() {
    let t = test_db!("saved_diffs_are_private_and_deletable");
    let server = online(&t, &[]);
    let app = &server.app;
    let (_, key) = new_key(&server.state, &[]).await;
    let (_, other) = new_key(&server.state, &[]).await;
    let saved = post_json(
        app,
        "/v1/diff?save=true",
        Some(&key),
        &text_diff("x\ny\n", "x\nz\n"),
    )
    .await;
    assert_eq!(saved.status, StatusCode::CREATED);
    let location = saved.header("location").to_owned();
    assert!(location.starts_with("/v1/diffs/"), "{location}");
    let fetched = get(app, &location, Some(&key)).await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.body, saved.body, "stored byte for byte");
    assert_eq!(fetched.header("content-type"), "application/json");
    assert_eq!(
        get(app, &location, Some(&other)).await.code(),
        "diff-not-found"
    );
    let delete = |k: String| {
        let request = request(Method::DELETE, &location)
            .header(header::AUTHORIZATION, format!("Bearer {k}"))
            .body(Body::empty())
            .unwrap();
        send(app, request)
    };
    assert_eq!(delete(other.clone()).await.code(), "diff-not-found");

    let id: uuid::Uuid = location.trim_start_matches("/v1/diffs/").parse().unwrap();
    let diff = diffyman_db::diffs::get(t.db.pool(), id)
        .await
        .unwrap()
        .unwrap();
    let stored: Vec<BlobId> = [
        Some(diff.document_blob.clone()),
        diff.left_blob.clone(),
        diff.right_blob.clone(),
    ]
    .into_iter()
    .flatten()
    .map(|b| b.parse().unwrap())
    .collect();
    assert_eq!(stored.len(), 3);
    for blob in &stored {
        assert!(server.state.store.head(*blob).await.unwrap().is_some());
    }
    assert_eq!(delete(key.clone()).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        get(app, &location, Some(&key)).await.code(),
        "diff-not-found"
    );
    for blob in &stored {
        assert!(
            server.state.store.head(*blob).await.unwrap().is_none(),
            "object removed"
        );
        assert!(
            blobs::get(t.db.pool(), &blob.to_hex())
                .await
                .unwrap()
                .is_none(),
            "row removed"
        );
    }
    assert_eq!(delete(key).await.code(), "diff-not-found");
    drop(server);
    t.finish().await;
}

async fn upload(app: &Router, key: &str, bytes: &[u8]) -> String {
    let reply = post_bytes(app, "/v1/blobs", Some(key), bytes).await;
    assert!(
        reply.status.is_success(),
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    reply.json()["blobId"].as_str().unwrap().to_owned()
}

async fn wait_for(app: &Router, key: &str, location: &str, done: &[&str]) -> Value {
    for _ in 0..400 {
        let job = get(app, location, Some(key)).await.json();
        if done.contains(&job["status"].as_str().unwrap_or("")) {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("job at {location} did not finish");
}

#[tokio::test]
async fn jobs_run_end_to_end() {
    let t = test_db!("jobs_run_end_to_end");
    let server = online(&t, &[("DIFFYMAN_JOB_MEMORY_BYTES", "1048576")]);
    let app = &server.app;
    let (key_id, key) = new_key(&server.state, &[]).await;
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let running = tokio::spawn(worker::run(server.state.clone(), stopped));

    let (left, right) = (
        b"alpha\nbeta\ngamma\n".as_slice(),
        b"alpha\nBETA\ngamma\n".as_slice(),
    );
    let first = post_bytes(app, "/v1/blobs", Some(&key), left).await;
    assert_eq!(first.status, StatusCode::CREATED);
    let again = post_bytes(app, "/v1/blobs", Some(&key), left).await;
    assert_eq!(again.status, StatusCode::OK, "same bytes, same blob");
    let left_id = first.json()["blobId"].as_str().unwrap().to_owned();
    assert_eq!(left_id, BlobId::of(left).to_hex());
    let wrong = format!("/v1/blobs?sha256={left_id}");
    assert_eq!(
        post_bytes(app, &wrong, Some(&key), right).await.code(),
        "hash-mismatch"
    );
    let right_id = upload(app, &key, right).await;

    let options = json!({"granularity": "word", "context": 1});
    let body = json!({
        "left": {"blob": left_id, "name": "old.txt"},
        "right": {"blob": right_id, "name": "new.txt"},
        "options": options,
    });
    let created = post_json(app, "/v1/jobs", Some(&key), &body).await;
    assert_eq!(
        created.status,
        StatusCode::ACCEPTED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    let location = created.header("location").to_owned();
    let job = wait_for(app, &key, &location, &["succeeded", "failed"]).await;
    assert_eq!(job["status"], "succeeded", "{job}");
    assert_eq!(job["attempts"], 1);
    let result = job["result"].as_str().unwrap().to_owned();
    let document = get(app, &result, Some(&key)).await;
    assert_eq!(document.status, StatusCode::OK);
    let engine = diffyman::diff_inputs_with(
        diffyman::Input::named(left, "old.txt"),
        diffyman::Input::named(right, "new.txt"),
        &options.to_string(),
        &diffyman::Settings::default(),
    )
    .unwrap();
    assert_eq!(conformance(&document.body), conformance(engine.as_bytes()));

    // A job from one side of the saved diff and a new blob.
    let diff_id = job["diffId"].as_str().unwrap();
    let newer = upload(app, &key, b"alpha\nBETA\ndelta\n").await;
    let body = json!({
        "left": {"diff": diff_id, "side": "right", "name": "new.txt"},
        "right": {"blob": newer, "name": "newer.txt"},
    });
    let chained = post_json(app, "/v1/jobs", Some(&key), &body).await;
    assert_eq!(chained.status, StatusCode::ACCEPTED);
    let job = wait_for(
        app,
        &key,
        chained.header("location"),
        &["succeeded", "failed"],
    )
    .await;
    assert_eq!(job["status"], "succeeded", "{job}");
    assert_eq!(job["left"]["blob"], right_id);

    // Input errors fail at once, without retries.
    let missing = json!({"left": {"blob": "0".repeat(64)}, "right": {"blob": right_id}});
    assert_eq!(
        post_json(app, "/v1/jobs", Some(&key), &missing)
            .await
            .code(),
        "blob-not-found"
    );
    let bad =
        json!({"left": {"blob": left_id}, "right": {"blob": right_id}, "options": {"context": -1}});
    assert_eq!(
        post_json(app, "/v1/jobs", Some(&key), &bad).await.code(),
        "invalid-options"
    );
    let neither = json!({"left": {"blob": left_id, "diff": diff_id, "side": "left"}, "right": {"blob": right_id}});
    assert_eq!(
        post_json(app, "/v1/jobs", Some(&key), &neither)
            .await
            .code(),
        "invalid-request"
    );
    let big = upload(app, &key, &vec![b'x'; 200 << 10]).await;
    let big2 = upload(app, &key, &vec![b'y'; 200 << 10]).await;
    let too_big = json!({"left": {"blob": big}, "right": {"blob": big2}});
    assert_eq!(
        post_json(app, "/v1/jobs", Some(&key), &too_big)
            .await
            .code(),
        "job-too-large"
    );

    // A blob row whose object is gone: not an infrastructure error.
    let ghost = BlobId::of(b"ghost").to_hex();
    let tomorrow = chrono::Utc::now() + chrono::TimeDelta::days(1);
    blobs::upsert(t.db.pool(), &ghost, 5, None, tomorrow)
        .await
        .unwrap();
    blobs::grant(t.db.pool(), &ghost, Some(key_id), None, tomorrow)
        .await
        .unwrap();
    let body = json!({"left": {"blob": ghost}, "right": {"blob": right_id}});
    let created = post_json(app, "/v1/jobs", Some(&key), &body).await;
    let job = wait_for(
        app,
        &key,
        created.header("location"),
        &["succeeded", "failed"],
    )
    .await;
    assert_eq!(job["status"], "failed");
    assert_eq!(job["error"]["code"], "blob-not-found");
    assert_eq!(job["attempts"], 1, "not retried");

    // Another key sees none of it.
    let (_, other) = new_key(&server.state, &[]).await;
    assert_eq!(
        get(app, &location, Some(&other)).await.code(),
        "job-not-found"
    );
    assert_eq!(
        get(app, &result, Some(&other)).await.code(),
        "diff-not-found"
    );

    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(10), running)
        .await
        .unwrap()
        .unwrap();
    drop(server);
    t.finish().await;
}

#[tokio::test]
async fn jobs_can_be_cancelled_queued_or_running() {
    let t = test_db!("jobs_can_be_cancelled_queued_or_running");
    let server = online(&t, &[]);
    let app = &server.app;
    let (_, key) = new_key(&server.state, &[]).await;
    let left = upload(app, &key, b"a\n").await;
    let right = upload(app, &key, b"b\n").await;
    let body = json!({"left": {"blob": left}, "right": {"blob": right}});

    // Queued (no worker runs): cancelled at once.
    let created = post_json(app, "/v1/jobs", Some(&key), &body).await;
    let cancel = format!("{}/cancel", created.header("location"));
    let reply = send(
        app,
        request(Method::POST, &cancel)
            .header(header::AUTHORIZATION, format!("Bearer {key}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["status"], "cancelled");
    let again = send(
        app,
        request(Method::POST, &cancel)
            .header(header::AUTHORIZATION, format!("Bearer {key}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        (again.status, again.code().as_str()),
        (StatusCode::CONFLICT, "job-finished")
    );

    // Running: flagged, and the worker stops at its heartbeat.
    let created = post_json(app, "/v1/jobs", Some(&key), &body).await;
    let location = created.header("location").to_owned();
    let lease = server.state.config.limits.job_lease;
    let claimed = jobs::claim(t.db.pool(), &server.state.worker_id, lease, 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    let cancel = format!("{location}/cancel");
    let reply = send(
        app,
        request(Method::POST, &cancel)
            .header(header::AUTHORIZATION, format!("Bearer {key}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::ACCEPTED);
    assert_eq!(reply.json()["status"], "running");
    assert_eq!(reply.json()["cancelRequested"], true);
    let job = claimed.into_iter().next().unwrap();
    worker::run_claimed(server.state.clone(), job.clone()).await;
    let after = jobs::get(t.db.pool(), job.id).await.unwrap().unwrap();
    assert_eq!(after.status, JobStatus::Cancelled);
    assert!(after.diff_id.is_none());
    drop(server);
    t.finish().await;
}

#[tokio::test]
async fn retention_sweeps_expired_and_orphaned_content() {
    let t = test_db!("retention_sweeps_expired_and_orphaned_content");
    let server = online(&t, &[]);
    let app = &server.app;
    let (_, key) = new_key(&server.state, &[]).await;
    let saved = post_json(
        app,
        "/v1/diff?save=true",
        Some(&key),
        &text_diff("1\n", "2\n"),
    )
    .await;
    let location = saved.header("location").to_owned();
    let kept = upload(app, &key, b"still wanted").await;
    let store: Arc<dyn BlobStore> = server.state.store.clone();

    let nothing = sweeper::sweep_once(&server.state).await.unwrap();
    assert_eq!(nothing, sweeper::SweepReport::default());

    // Expire the saved diff and its blobs, but not the other upload.
    sqlx::query("UPDATE diffs SET expires_at = now() - interval '1 second'")
        .execute(t.db.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE blobs SET expires_at = now() - interval '1 second' WHERE sha256 <> $1")
        .bind(&kept)
        .execute(t.db.pool())
        .await
        .unwrap();
    assert_eq!(
        get(app, &location, Some(&key)).await.code(),
        "diff-not-found",
        "expired is gone"
    );
    let report = sweeper::sweep_once(&server.state).await.unwrap();
    assert_eq!(report.diffs, 1);
    assert_eq!(report.blobs, 3, "document and both inputs");
    let kept_id: BlobId = kept.parse().unwrap();
    assert!(store.head(kept_id).await.unwrap().is_some());
    let left = BlobId::of(b"1\n");
    assert!(store.head(left).await.unwrap().is_none());

    // An object without a row is removed once older than the grace period.
    let orphan = store
        .put_bytes(Bytes::from_static(b"orphan"), None)
        .await
        .unwrap();
    let report = sweeper::sweep_once(&server.state).await.unwrap();
    assert_eq!(report.orphans, 0, "too young");
    let later = chrono::Utc::now() + chrono::TimeDelta::minutes(1);
    let (orphans, _) = sweeper::sweep_orphans(&server.state, later).await.unwrap();
    assert_eq!(orphans, 1);
    assert!(store.head(orphan.id).await.unwrap().is_none());
    assert!(store.head(kept_id).await.unwrap().is_some(), "has a row");

    let audits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit WHERE action = 'retention.sweep'")
            .fetch_one(t.db.pool())
            .await
            .unwrap();
    assert_eq!(audits, 1);
    drop(server);
    t.finish().await;
}

#[tokio::test]
async fn auth_none_mode_shares_one_owner() {
    let t = test_db!("auth_none_mode_shares_one_owner");
    let server = online(&t, &[("DIFFYMAN_AUTH_MODE", "none")]);
    let saved = post_json(&server.app, "/v1/diff?save=1", None, &text_diff("a", "b")).await;
    assert_eq!(saved.status, StatusCode::CREATED);
    let fetched = get(&server.app, saved.header("location"), None).await;
    assert_eq!(fetched.body, saved.body);
    assert_eq!(
        get(&server.app, "/healthz", None).await.status,
        StatusCode::OK
    );
    drop(server);
    t.finish().await;
}

/// Starts a job on `left` and `right` and returns the reply.
async fn post_job(app: &Router, key: &str, left: &str, right: &str) -> Reply {
    let body = json!({"left": {"blob": left}, "right": {"blob": right}});
    post_json(app, "/v1/jobs", Some(key), &body).await
}

async fn delete_diff(app: &Router, key: &str, location: &str) -> Reply {
    let request = request(Method::DELETE, location)
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap();
    send(app, request).await
}

#[tokio::test]
async fn blobs_belong_to_the_key_that_uploaded_them() {
    let t = test_db!("blobs_belong_to_the_key_that_uploaded_them");
    let server = online(&t, &[]);
    let app = &server.app;
    let (_, a) = new_key(&server.state, &[]).await;
    let (_, b) = new_key(&server.state, &[]).await;
    let bytes = b"key A's private upload\n";
    let first = post_bytes(app, "/v1/blobs", Some(&a), bytes).await;
    assert_eq!(first.status, StatusCode::CREATED);
    let x = first.json()["blobId"].as_str().unwrap().to_owned();
    assert_eq!(
        post_bytes(app, "/v1/blobs", Some(&a), bytes).await.status,
        StatusCode::OK
    );

    // Key B cannot use the blob, and cannot tell that it exists.
    let unknown = BlobId::of(b"never uploaded").to_hex();
    let stranger = post_job(app, &b, &x, &x).await;
    let missing = post_job(app, &b, &unknown, &unknown).await;
    assert_eq!(
        (stranger.status, stranger.code()),
        (missing.status, missing.code())
    );
    assert_eq!(stranger.code(), "blob-not-found");

    // Uploading the same bytes is new for key B, and then B may use them.
    let own = post_bytes(app, "/v1/blobs", Some(&b), bytes).await;
    assert_eq!(own.status, StatusCode::CREATED, "no sign of key A's upload");
    assert_eq!(own.json()["blobId"], x.as_str());
    assert_eq!(post_job(app, &b, &x, &x).await.status, StatusCode::ACCEPTED);
    drop(server);
    t.finish().await;
}

#[tokio::test]
async fn deleting_a_diff_keeps_another_keys_upload() {
    let t = test_db!("deleting_a_diff_keeps_another_keys_upload");
    let server = online(&t, &[]);
    let app = &server.app;
    let (_, key_a) = new_key(&server.state, &[]).await;
    let (_, key_b) = new_key(&server.state, &[]).await;
    let store = server.state.store.clone();

    // B uploads first, then A saves a diff of the same bytes and deletes it.
    let before = b"uploaded by B before A's diff\n";
    let x = upload(app, &key_b, before).await;
    let saved = post_json(
        app,
        "/v1/diff?save=true",
        Some(&key_a),
        &json!({"left": {"text": std::str::from_utf8(before).unwrap()}, "right": {"text": "1\n"}}),
    )
    .await;
    assert_eq!(saved.status, StatusCode::CREATED);
    let location = saved.header("location").to_owned();
    assert_eq!(
        delete_diff(app, &key_a, &location).await.status,
        StatusCode::NO_CONTENT
    );
    let x_id: BlobId = x.parse().unwrap();
    assert!(
        store.head(x_id).await.unwrap().is_some(),
        "B's object stays"
    );
    assert!(blobs::get(t.db.pool(), &x).await.unwrap().is_some());
    assert_eq!(
        post_job(app, &key_b, &x, &x).await.status,
        StatusCode::ACCEPTED
    );

    // A saves first, then B uploads the same bytes, then A deletes.
    let after = b"uploaded by B after A's diff\n";
    let saved = post_json(
        app,
        "/v1/diff?save=true",
        Some(&key_a),
        &json!({"left": {"text": std::str::from_utf8(after).unwrap()}, "right": {"text": "2\n"}}),
    )
    .await;
    let location = saved.header("location").to_owned();
    let y = post_bytes(app, "/v1/blobs", Some(&key_b), after).await;
    assert_eq!(y.status, StatusCode::CREATED);
    let y = y.json()["blobId"].as_str().unwrap().to_owned();
    assert_eq!(
        delete_diff(app, &key_a, &location).await.status,
        StatusCode::NO_CONTENT
    );
    let y_id: BlobId = y.parse().unwrap();
    assert!(
        store.head(y_id).await.unwrap().is_some(),
        "B's object stays"
    );
    assert_eq!(
        post_job(app, &key_b, &y, &y).await.status,
        StatusCode::ACCEPTED
    );
    // The parts only A's diff held are gone.
    let right_id = BlobId::of(b"2\n");
    assert!(store.head(right_id).await.unwrap().is_none());
    drop(server);
    t.finish().await;
}

/// A blob store whose deletes of chosen blobs fail.
#[derive(Debug)]
struct RefusingDeletes {
    inner: Arc<dyn BlobStore>,
    refused: std::sync::Mutex<Vec<BlobId>>,
}

type StoreFuture<'a, T> = futures::future::BoxFuture<'a, diffyman_storage::Result<T>>;

impl BlobStore for RefusingDeletes {
    fn max_blob_bytes(&self) -> u64 {
        self.inner.max_blob_bytes()
    }

    fn put_bytes(
        &self,
        data: Bytes,
        expected: Option<BlobId>,
    ) -> StoreFuture<'_, diffyman_storage::PutOutcome> {
        self.inner.put_bytes(data, expected)
    }

    fn put_stream(
        &self,
        data: diffyman_storage::InputStream,
        expected: Option<BlobId>,
    ) -> StoreFuture<'_, diffyman_storage::PutOutcome> {
        self.inner.put_stream(data, expected)
    }

    fn get(
        &self,
        id: BlobId,
    ) -> StoreFuture<'_, (diffyman_storage::BlobMeta, diffyman_storage::ByteStream)> {
        self.inner.get(id)
    }

    fn get_range(
        &self,
        id: BlobId,
        range: std::ops::Range<u64>,
    ) -> StoreFuture<'_, diffyman_storage::ByteStream> {
        self.inner.get_range(id, range)
    }

    fn head(&self, id: BlobId) -> StoreFuture<'_, Option<diffyman_storage::BlobMeta>> {
        self.inner.head(id)
    }

    fn delete(&self, id: BlobId) -> StoreFuture<'_, ()> {
        if self.refused.lock().unwrap().contains(&id) {
            return Box::pin(async {
                Err(diffyman_storage::StorageError::Backend("refused".into()))
            });
        }
        self.inner.delete(id)
    }

    fn list(
        &self,
        area: diffyman_storage::Area,
    ) -> futures::stream::BoxStream<'_, diffyman_storage::Result<diffyman_storage::StoredObject>>
    {
        self.inner.list(area)
    }

    fn delete_object<'a>(&'a self, kind: &'a diffyman_storage::ObjectKind) -> StoreFuture<'a, ()> {
        self.inner.delete_object(kind)
    }
}

#[tokio::test]
async fn a_failed_object_delete_leaves_an_orphan_not_a_broken_diff() {
    let t = test_db!("a_failed_object_delete_leaves_an_orphan_not_a_broken_diff");
    let home = tempfile::tempdir().unwrap();
    let config = config(home.path(), &[]);
    let inner = diffyman_storage::open("memory://", StoreOptions::default()).unwrap();
    let store = Arc::new(RefusingDeletes {
        inner,
        refused: std::sync::Mutex::new(Vec::new()),
    });
    let (state, app) = build_with(config, t.db.clone(), store.clone());
    let (_, key) = new_key(&state, &[]).await;
    let saved = post_json(
        &app,
        "/v1/diff?save=true",
        Some(&key),
        &text_diff("orphan left\n", "orphan right\n"),
    )
    .await;
    let location = saved.header("location").to_owned();
    let left = BlobId::of(b"orphan left\n");
    let right = BlobId::of(b"orphan right\n");
    store.refused.lock().unwrap().push(left);

    // The rows are deleted and committed; the object that could not be
    // deleted is left without a row.
    assert_eq!(
        delete_diff(&app, &key, &location).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(&app, &location, Some(&key)).await.code(),
        "diff-not-found"
    );
    for blob in [left, right] {
        assert!(
            blobs::get(t.db.pool(), &blob.to_hex())
                .await
                .unwrap()
                .is_none()
        );
    }
    assert!(store.head(right).await.unwrap().is_none());
    assert!(store.head(left).await.unwrap().is_some(), "an orphan");

    // The orphan sweep removes it once the store allows it.
    store.refused.lock().unwrap().clear();
    let later = chrono::Utc::now() + chrono::TimeDelta::minutes(1);
    let (orphans, _) = sweeper::sweep_orphans(&state, later).await.unwrap();
    assert_eq!(orphans, 1);
    assert!(store.head(left).await.unwrap().is_none());
    drop((state, app));
    t.finish().await;
}

#[tokio::test]
async fn a_job_slot_is_held_until_the_result_is_committed() {
    let t = test_db!("a_job_slot_is_held_until_the_result_is_committed");
    let server = online(&t, &[]);
    let app = &server.app;
    let (_, key) = new_key(&server.state, &[]).await;
    let left = upload(app, &key, b"slot\nleft\n").await;
    let right = upload(app, &key, b"slot\nright\n").await;
    let created = post_job(app, &key, &left, &right).await;
    assert_eq!(created.status, StatusCode::ACCEPTED);
    let lease = server.state.config.limits.job_lease;
    let job = jobs::claim(t.db.pool(), &server.state.worker_id, lease, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    // The slot is a channel end: the receiver wakes the moment it is dropped.
    let (slot, released) = tokio::sync::oneshot::channel::<()>();
    let running = tokio::spawn(worker::run_claimed_with_slot(
        server.state.clone(),
        job.clone(),
        slot,
    ));
    let _ = released.await;
    let at_release = jobs::get(t.db.pool(), job.id).await.unwrap().unwrap();
    assert_eq!(
        at_release.status,
        JobStatus::Succeeded,
        "the result was stored and committed before the slot was free"
    );
    running.await.unwrap();
    drop(server);
    t.finish().await;
}
