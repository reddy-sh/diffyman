//! Setup shared by the request-rule tests: a configuration on a memory
//! store, a server with or without a test database, and request helpers.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use bytes::Bytes;
use diffyman_db::Db;
use diffyman_server::config::{Config, Sources};
use diffyman_server::{AppState, Tuning, keys};
use diffyman_storage::{BlobStore, StoreOptions};
use serde_json::Value;
use tower::ServiceExt;

pub const HOST: &str = "127.0.0.1:8080";

/// A configuration on a memory store under `home`, with `vars` on top.
pub fn config(home: &Path, vars: &[(&str, &str)]) -> Config {
    let mut all: BTreeMap<String, String> = [
        ("DIFFYMAN_PG_PASSWORD", "unused"),
        ("DIFFYMAN_STORE_URL", "memory://"),
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

pub struct Server {
    pub state: AppState,
    pub app: Router,
    _home: tempfile::TempDir,
}

fn build(home: tempfile::TempDir, config: Config, db: Db, store: Arc<dyn BlobStore>) -> Server {
    let tuning = Tuning {
        poll_interval: Duration::from_millis(50),
        retry_delay: Duration::from_millis(100),
        shutdown_grace: Duration::from_secs(2),
    };
    let state = AppState::new(config, db, store, tuning);
    let app = diffyman_server::router(state.clone());
    Server {
        state,
        app,
        _home: home,
    }
}

/// The memory store of a configuration.
pub fn memory_store() -> Arc<dyn BlobStore> {
    diffyman_storage::open("memory://", StoreOptions::default()).unwrap()
}

/// A server without authentication whose database is never reachable: any
/// request that reaches a query gets `503 db-unavailable`.
pub fn offline(vars: &[(&str, &str)]) -> Server {
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
    build(home, config, Db::from_pool(pool), memory_store())
}

/// A server on a test database and `store`.
pub fn online(db: Db, store: Arc<dyn BlobStore>, vars: &[(&str, &str)]) -> Server {
    let home = tempfile::tempdir().unwrap();
    let config = config(home.path(), vars);
    build(home, config, db, store)
}

/// A new API key with every scope.
pub async fn new_key(state: &AppState) -> String {
    let (_, secret) = keys::create(&state.db, "test", &[]).await.unwrap();
    secret.expose().to_owned()
}

pub struct Reply {
    pub status: StatusCode,
    pub body: Bytes,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|_| panic!("not JSON ({}): {}", self.status, self.text()))
    }

    pub fn code(&self) -> String {
        self.json()["error"]["code"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Sends a request with `body` of `content_type`, as `key` when given.
pub async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    key: Option<&str>,
    content_type: &str,
    body: Vec<u8>,
) -> Reply {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, HOST)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, body.len());
    if let Some(key) = key {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Reply { status, body }
}

pub async fn post_json(app: &Router, uri: &str, key: Option<&str>, body: &Value) -> Reply {
    let text = body.to_string().into_bytes();
    send(app, Method::POST, uri, key, "application/json", text).await
}

pub async fn get(app: &Router, uri: &str, key: Option<&str>) -> Reply {
    send(app, Method::GET, uri, key, "application/json", Vec::new()).await
}

/// Uploads `bytes` as a blob and returns its ID.
pub async fn upload(app: &Router, key: &str, bytes: &[u8]) -> String {
    let reply = send(
        app,
        Method::POST,
        "/v1/blobs",
        Some(key),
        "application/octet-stream",
        bytes.to_vec(),
    )
    .await;
    assert!(reply.status.is_success(), "{}", reply.text());
    reply.json()["blobId"].as_str().unwrap().to_owned()
}
