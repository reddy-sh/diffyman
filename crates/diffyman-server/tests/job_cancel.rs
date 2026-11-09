//! A cancel requested while a job runs wins over the result the engine
//! produces after it (`docs/api/README.md`, `POST /v1/jobs/{id}/cancel`):
//! the job ends `cancelled` and no saved diff is left. Needs the test
//! database and skips without one.

#[allow(dead_code)]
#[path = "../../diffyman-db/tests/common/mod.rs"]
mod common;
mod support;

use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::{Method, StatusCode};
use bytes::Bytes;
use diffyman_db::jobs::{self, JobStatus};
use diffyman_server::worker;
use diffyman_storage::{
    Area, BlobId, BlobMeta, BlobStore, ByteStream, InputStream, ObjectKind, PutOutcome,
    StoredObject,
};
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use serde_json::json;
use support::{get, new_key, online, post_json, send, upload};
use tokio::sync::oneshot;

type StoreFuture<'a, T> = BoxFuture<'a, diffyman_storage::Result<T>>;

/// Where a read pauses: the blob, a signal that the read was reached, and
/// the signal to go on.
type Pause = (BlobId, oneshot::Sender<()>, oneshot::Receiver<()>);

/// A memory store whose next read of one blob waits until it is released,
/// so that a test can act while a job is running.
#[derive(Debug)]
struct PausingStore {
    inner: Arc<dyn BlobStore>,
    pause: Mutex<Option<Pause>>,
}

impl BlobStore for PausingStore {
    fn max_blob_bytes(&self) -> u64 {
        self.inner.max_blob_bytes()
    }

    fn put_bytes(&self, data: Bytes, expected: Option<BlobId>) -> StoreFuture<'_, PutOutcome> {
        self.inner.put_bytes(data, expected)
    }

    fn put_stream(
        &self,
        data: InputStream,
        expected: Option<BlobId>,
    ) -> StoreFuture<'_, PutOutcome> {
        self.inner.put_stream(data, expected)
    }

    fn get(&self, id: BlobId) -> StoreFuture<'_, (BlobMeta, ByteStream)> {
        let pause = {
            let mut pause = self.pause.lock().unwrap();
            if pause.as_ref().is_some_and(|(blob, _, _)| *blob == id) {
                pause.take()
            } else {
                None
            }
        };
        Box::pin(async move {
            if let Some((_, reached, resume)) = pause {
                let _ = reached.send(());
                let _ = resume.await;
            }
            self.inner.get(id).await
        })
    }

    fn get_range(&self, id: BlobId, range: Range<u64>) -> StoreFuture<'_, ByteStream> {
        self.inner.get_range(id, range)
    }

    fn head(&self, id: BlobId) -> StoreFuture<'_, Option<BlobMeta>> {
        self.inner.head(id)
    }

    fn delete(&self, id: BlobId) -> StoreFuture<'_, ()> {
        self.inner.delete(id)
    }

    fn list(&self, area: Area) -> BoxStream<'_, diffyman_storage::Result<StoredObject>> {
        self.inner.list(area)
    }

    fn delete_object<'a>(&'a self, kind: &'a ObjectKind) -> StoreFuture<'a, ()> {
        self.inner.delete_object(kind)
    }
}

#[tokio::test]
async fn test_cancel_while_running_discards_the_result() {
    let Some(t) = common::test_db("test_cancel_while_running_discards_the_result").await else {
        return;
    };
    let store = Arc::new(PausingStore {
        inner: support::memory_store(),
        pause: Mutex::new(None),
    });
    // A long lease: no heartbeat comes between the cancel and the result.
    let server = online(
        t.db.clone(),
        store.clone(),
        &[("DIFFYMAN_JOB_LEASE_SECS", "120")],
    );
    let app = &server.app;
    let key = new_key(&server.state).await;
    let left = upload(app, &key, b"cancel\nleft\n").await;
    let right = upload(app, &key, b"cancel\nright\n").await;
    let created = post_json(
        app,
        "/v1/jobs",
        Some(&key),
        &json!({"left": {"blob": left}, "right": {"blob": right}}),
    )
    .await;
    assert_eq!(created.status, StatusCode::ACCEPTED, "{}", created.text());
    let location = format!("/v1/jobs/{}", created.json()["id"].as_str().unwrap());

    let (reached, at_read) = oneshot::channel();
    let (resume, resumed) = oneshot::channel();
    *store.pause.lock().unwrap() = Some((left.parse().unwrap(), reached, resumed));
    let lease = server.state.config.limits.job_lease;
    let job = jobs::claim(t.db.pool(), &server.state.worker_id, lease, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let running = tokio::spawn(worker::run_claimed(server.state.clone(), job));

    // The job is running (reading its input) when the cancel comes in.
    tokio::time::timeout(Duration::from_secs(10), at_read)
        .await
        .expect("the job reads its input")
        .unwrap();
    let cancel = format!("{location}/cancel");
    let reply = send(
        app,
        Method::POST,
        &cancel,
        Some(&key),
        "application/json",
        Vec::new(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::ACCEPTED, "{}", reply.text());
    assert_eq!(reply.json()["cancelRequested"], true);

    // The engine runs to the end; its result must not be recorded.
    resume.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(20), running)
        .await
        .expect("the job finishes")
        .unwrap();
    let job = get(app, &location, Some(&key)).await.json();
    assert_eq!(job["status"], "cancelled", "{job}");
    assert_eq!(job["result"], serde_json::Value::Null);
    assert_eq!(job["diffId"], serde_json::Value::Null);
    let id = job["id"].as_str().unwrap().parse().unwrap();
    let stored = jobs::get(t.db.pool(), id).await.unwrap().unwrap();
    assert_eq!(stored.status, JobStatus::Cancelled);
    assert_eq!((stored.result_blob, stored.diff_id), (None, None));
    let saved: i64 = sqlx::query_scalar("SELECT count(*) FROM diffs")
        .fetch_one(t.db.pool())
        .await
        .unwrap();
    assert_eq!(saved, 0, "no saved diff is left");
    drop(server);
    t.finish().await;
}
