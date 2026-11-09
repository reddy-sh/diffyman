//! Integration tests against a real PostgreSQL 18, each in its own schema of
//! the `diffyman_test` database. They skip (and pass) when no database is
//! reachable; see `common/mod.rs`.

mod common;

use std::collections::HashSet;
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use diffyman_db::jobs::{self, Heartbeat, JobStatus, NewJob};
use diffyman_db::records::{self, Actor, AuditEvent, Usage};
use diffyman_db::{DbError, PgPool, api_keys, blobs, diffs, retention};
use serde_json::json;
use uuid::Uuid;

macro_rules! test_db {
    ($name:literal) => {
        match common::test_db($name).await {
            Some(t) => t,
            None => return,
        }
    };
}

fn hex(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn later() -> chrono::DateTime<Utc> {
    Utc::now() + TimeDelta::days(30)
}

fn earlier() -> chrono::DateTime<Utc> {
    Utc::now() - TimeDelta::seconds(1)
}

async fn blob(pool: &PgPool, n: u8) -> String {
    blobs::upsert(pool, &hex(n), 10, Some("text/plain"), later())
        .await
        .unwrap();
    hex(n)
}

async fn job(pool: &PgPool, left: &str, right: &str) -> jobs::Job {
    let options = json!({"granularity": "word"});
    let new = NewJob {
        api_key_id: None,
        options: &options,
        left_blob: left,
        right_blob: right,
        left_name: Some("a.txt"),
        right_name: Some("b.txt"),
        max_attempts: 3,
        expires_at: later(),
    };
    jobs::create(pool, &new).await.unwrap()
}

#[tokio::test]
async fn migrations_are_idempotent_and_ping_works() {
    let t = test_db!("migrations_are_idempotent_and_ping_works");
    t.db.migrate().await.unwrap();
    t.db.ping().await.unwrap();
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables
         WHERE table_schema = current_schema() ORDER BY 1",
    )
    .fetch_all(t.db.pool())
    .await
    .unwrap();
    assert_eq!(
        tables,
        [
            "_sqlx_migrations",
            "api_keys",
            "audit",
            "blob_grants",
            "blobs",
            "diffs",
            "jobs",
            "usage"
        ]
    );
    t.finish().await;
}

#[tokio::test]
async fn api_keys_store_only_prefix_and_hash() {
    let t = test_db!("api_keys_store_only_prefix_and_hash");
    let pool = t.db.pool();
    let scopes = vec!["diff".to_owned(), "diffs:delete".to_owned()];
    let (key, secret) = api_keys::create(pool, "laptop", &scopes).await.unwrap();
    assert_eq!(key.prefix, secret.prefix());
    assert!(key.has_scope("diffs:delete") && !key.has_scope("admin"));
    assert!(key.last_used_at.is_none());

    // No column holds the secret.
    let dump: String = sqlx::query_scalar("SELECT row_to_json(k)::text FROM api_keys k")
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(!dump.contains(&secret.expose()[12..]));

    let found = api_keys::authenticate(pool, secret.expose())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, key.id);
    let again = api_keys::authenticate(pool, secret.expose())
        .await
        .unwrap()
        .unwrap();
    assert!(again.last_used_at.is_some());

    let mut wrong = secret.expose().to_owned();
    let last = if wrong.ends_with('0') { "1" } else { "0" };
    wrong.replace_range(67.., last);
    assert!(
        api_keys::authenticate(pool, &wrong)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        api_keys::authenticate(pool, "garbage")
            .await
            .unwrap()
            .is_none()
    );

    let bad_scope = api_keys::create(pool, "x", &["Not A Scope".to_owned()])
        .await
        .unwrap_err();
    assert_eq!(bad_scope.code(), "invalid-value");

    assert!(api_keys::revoke(pool, key.id).await.unwrap());
    assert!(!api_keys::revoke(pool, key.id).await.unwrap());
    assert!(
        api_keys::authenticate(pool, secret.expose())
            .await
            .unwrap()
            .is_none()
    );
    let listed = api_keys::list(pool).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].revoked_at.is_some());
    t.finish().await;
}

#[tokio::test]
async fn blob_upsert_extends_expiry() {
    let t = test_db!("blob_upsert_extends_expiry");
    let pool = t.db.pool();
    let soon = Utc::now() + TimeDelta::days(1);
    let (first, inserted) = blobs::upsert(pool, &hex(1), 5, None, soon).await.unwrap();
    assert!(inserted);
    let (second, inserted) = blobs::upsert(pool, &hex(1), 5, None, later())
        .await
        .unwrap();
    assert!(!inserted);
    assert!(second.expires_at > first.expires_at);
    let (third, _) = blobs::upsert(pool, &hex(1), 5, None, soon).await.unwrap();
    assert_eq!(third.expires_at, second.expires_at);
    assert_eq!(blobs::get(pool, &hex(1)).await.unwrap(), Some(third));
    assert_eq!(blobs::get(pool, &hex(2)).await.unwrap(), None);
    assert_eq!(
        blobs::existing(pool, &[hex(1), hex(2)]).await.unwrap(),
        vec![hex(1)]
    );
    let err = blobs::upsert(pool, "ABC", 5, None, soon).await.unwrap_err();
    assert_eq!(err.code(), "invalid-value");
    let err = blobs::upsert(pool, &hex(3), -1, None, soon)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "invalid-value");
    t.finish().await;
}

#[tokio::test]
async fn job_lifecycle_with_leases() {
    let t = test_db!("job_lifecycle_with_leases");
    let pool = t.db.pool();
    let options = json!({});
    let missing = NewJob {
        api_key_id: None,
        options: &options,
        left_blob: &hex(9),
        right_blob: &hex(9),
        left_name: None,
        right_name: None,
        max_attempts: 1,
        expires_at: later(),
    };
    assert!(matches!(
        jobs::create(pool, &missing).await,
        Err(DbError::MissingReference)
    ));

    let (l, r) = (blob(pool, 1).await, blob(pool, 2).await);
    let created = job(pool, &l, &r).await;
    assert_eq!(created.status, JobStatus::Queued);
    assert_eq!(created.id.get_version_num(), 7);
    assert_eq!(created.options, json!({"granularity": "word"}));

    let lease = Duration::from_secs(30);
    let claimed = jobs::claim(pool, "w1", lease, 10).await.unwrap();
    assert_eq!(claimed.len(), 1);
    let running = &claimed[0];
    assert_eq!((running.status, running.attempts), (JobStatus::Running, 1));
    assert_eq!(running.lease_owner.as_deref(), Some("w1"));
    assert!(running.lease_expires_at.unwrap() > Utc::now() + TimeDelta::seconds(20));
    assert!(jobs::claim(pool, "w2", lease, 10).await.unwrap().is_empty());

    assert_eq!(
        jobs::heartbeat(pool, created.id, "w1", lease)
            .await
            .unwrap(),
        Heartbeat::Continue
    );
    assert_eq!(
        jobs::heartbeat(pool, created.id, "w2", lease)
            .await
            .unwrap(),
        Heartbeat::Lost
    );

    let doc = blob(pool, 3).await;
    assert_eq!(
        jobs::succeed(pool, created.id, "w2", &doc, None)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        jobs::succeed(pool, created.id, "w1", &doc, None)
            .await
            .unwrap(),
        Some(JobStatus::Succeeded)
    );
    let done = jobs::get(pool, created.id).await.unwrap().unwrap();
    assert_eq!(done.status, JobStatus::Succeeded);
    assert!(done.status.is_final());
    assert_eq!(done.result_blob.as_deref(), Some(doc.as_str()));
    assert!(done.finished_at.is_some() && done.lease_owner.is_none());
    assert_eq!(
        jobs::succeed(pool, created.id, "w1", &doc, None)
            .await
            .unwrap(),
        None
    );
    assert_eq!(jobs::get(pool, Uuid::nil()).await.unwrap(), None);
    t.finish().await;
}

#[tokio::test]
async fn concurrent_claims_never_share_a_job() {
    let t = test_db!("concurrent_claims_never_share_a_job");
    let pool = t.db.pool().clone();
    let (l, r) = (blob(&pool, 1).await, blob(&pool, 2).await);
    let mut all = HashSet::new();
    for _ in 0..40 {
        all.insert(job(&pool, &l, &r).await.id);
    }
    let other = common::second_pool(&t).await;
    let worker = |pool: PgPool, name: &'static str| async move {
        let mut mine = Vec::new();
        loop {
            let batch = jobs::claim(&pool, name, Duration::from_secs(30), 3)
                .await
                .unwrap();
            if batch.is_empty() {
                return mine;
            }
            mine.extend(batch.into_iter().map(|j| j.id));
        }
    };
    let (mine_a, mine_b) = tokio::join!(
        tokio::spawn(worker(pool.clone(), "wa")),
        tokio::spawn(worker(other.clone(), "wb"))
    );
    let (mine_a, mine_b) = (mine_a.unwrap(), mine_b.unwrap());
    let mut seen = HashSet::new();
    for id in mine_a.iter().chain(&mine_b) {
        assert!(seen.insert(*id), "job {id} claimed twice");
    }
    assert_eq!(seen, all);
    other.close().await;
    t.finish().await;
}

#[tokio::test]
async fn failures_retry_then_fail() {
    let t = test_db!("failures_retry_then_fail");
    let pool = t.db.pool();
    let (l, r) = (blob(pool, 1).await, blob(pool, 2).await);
    let id = job(pool, &l, &r).await.id;
    let lease = Duration::from_secs(30);

    jobs::claim(pool, "w", lease, 1).await.unwrap();
    let status = jobs::fail(
        pool,
        id,
        "w",
        "internal",
        "boom",
        Some(Duration::from_secs(60)),
    )
    .await
    .unwrap();
    assert_eq!(status, Some(JobStatus::Queued));
    let queued = jobs::get(pool, id).await.unwrap().unwrap();
    assert_eq!(queued.error_code.as_deref(), Some("internal"));
    assert!(queued.run_after > Utc::now() + TimeDelta::seconds(50));
    assert!(
        jobs::claim(pool, "w", lease, 1).await.unwrap().is_empty(),
        "not before run_after"
    );

    sqlx::query("UPDATE jobs SET run_after = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    jobs::claim(pool, "w", lease, 1).await.unwrap();
    let status = jobs::fail(pool, id, "w", "limit-exceeded", "too big", None)
        .await
        .unwrap();
    assert_eq!(status, Some(JobStatus::Failed));
    let failed = jobs::get(pool, id).await.unwrap().unwrap();
    assert_eq!(
        (failed.attempts, failed.error_code.as_deref()),
        (2, Some("limit-exceeded"))
    );
    assert!(failed.finished_at.is_some());
    assert_eq!(
        jobs::fail(pool, id, "w", "x", "y", None).await.unwrap(),
        None
    );

    // The last attempt fails even when a retry is asked for.
    let id = job(pool, &l, &r).await.id;
    for attempt in 1..=3 {
        sqlx::query("UPDATE jobs SET run_after = now() WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        assert_eq!(
            jobs::claim(pool, "w", lease, 1).await.unwrap()[0].attempts,
            attempt
        );
        let status = jobs::fail(pool, id, "w", "internal", "boom", Some(Duration::ZERO)).await;
        let expected = if attempt < 3 {
            JobStatus::Queued
        } else {
            JobStatus::Failed
        };
        assert_eq!(status.unwrap(), Some(expected));
    }
    let err = jobs::fail(pool, Uuid::nil(), "w", "Bad Code", "", None)
        .await
        .unwrap();
    assert_eq!(err, None);
    t.finish().await;
}

#[tokio::test]
async fn cancellation() {
    let t = test_db!("cancellation");
    let pool = t.db.pool();
    let (l, r) = (blob(pool, 1).await, blob(pool, 2).await);
    let lease = Duration::from_secs(30);

    let queued = job(pool, &l, &r).await.id;
    assert_eq!(
        jobs::cancel(pool, queued).await.unwrap(),
        Some(JobStatus::Cancelled)
    );
    assert!(jobs::claim(pool, "w", lease, 5).await.unwrap().is_empty());

    let running = job(pool, &l, &r).await.id;
    jobs::claim(pool, "w", lease, 5).await.unwrap();
    assert_eq!(
        jobs::cancel(pool, running).await.unwrap(),
        Some(JobStatus::Running)
    );
    assert_eq!(
        jobs::heartbeat(pool, running, "w", lease).await.unwrap(),
        Heartbeat::Cancel
    );
    assert!(jobs::finish_cancelled(pool, running, "w").await.unwrap());
    let job = jobs::get(pool, running).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::Cancelled);
    assert_eq!(
        jobs::cancel(pool, running).await.unwrap(),
        Some(JobStatus::Cancelled)
    );
    assert_eq!(jobs::cancel(pool, Uuid::nil()).await.unwrap(), None);
    t.finish().await;
}

#[tokio::test]
async fn expired_leases_are_requeued_or_failed() {
    let t = test_db!("expired_leases_are_requeued_or_failed");
    let pool = t.db.pool();
    let (l, r) = (blob(pool, 1).await, blob(pool, 2).await);
    let retry = job(pool, &l, &r).await.id;
    let last = job(pool, &l, &r).await.id;
    sqlx::query("UPDATE jobs SET max_attempts = 1 WHERE id = $1")
        .bind(last)
        .execute(pool)
        .await
        .unwrap();
    let cancelled = job(pool, &l, &r).await.id;

    let claimed = jobs::claim(pool, "w", Duration::from_millis(1), 10)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 3);
    jobs::cancel(pool, cancelled).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let report = jobs::requeue_expired(pool, 10).await.unwrap();
    assert_eq!(
        report,
        jobs::RequeueReport {
            requeued: 1,
            failed: 1,
            cancelled: 1
        }
    );
    assert_eq!(
        jobs::get(pool, retry).await.unwrap().unwrap().status,
        JobStatus::Queued
    );
    let failed = jobs::get(pool, last).await.unwrap().unwrap();
    assert_eq!(failed.status, JobStatus::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("lease-expired"));
    assert_eq!(
        jobs::get(pool, cancelled).await.unwrap().unwrap().status,
        JobStatus::Cancelled
    );
    assert_eq!(
        jobs::heartbeat(pool, retry, "w", Duration::from_secs(1))
            .await
            .unwrap(),
        Heartbeat::Lost
    );
    assert_eq!(
        jobs::succeed(pool, retry, "w", &l, None).await.unwrap(),
        None
    );
    assert_eq!(
        jobs::requeue_expired(pool, 10).await.unwrap(),
        jobs::RequeueReport::default()
    );
    t.finish().await;
}

async fn saved_diff(pool: &PgPool, doc: &str, left: &str, right: &str) -> diffs::Diff {
    let stats = json!({"similarity": 0.5});
    let new = diffs::NewDiff {
        api_key_id: None,
        document_blob: doc,
        left_blob: Some(left),
        right_blob: Some(right),
        status: "different",
        kind: "text",
        stats: Some(&stats),
        expires_at: later(),
    };
    diffs::create(pool, &new).await.unwrap()
}

#[tokio::test]
async fn deleting_a_diff_removes_unreferenced_blobs() {
    let t = test_db!("deleting_a_diff_removes_unreferenced_blobs");
    let pool = t.db.pool();
    let (l, r, shared, doc) = (
        blob(pool, 1).await,
        blob(pool, 2).await,
        blob(pool, 3).await,
        blob(pool, 4).await,
    );
    let diff = saved_diff(pool, &doc, &l, &shared).await;
    let made_by = job(pool, &l, &r).await;
    jobs::claim(pool, "w", Duration::from_secs(30), 1)
        .await
        .unwrap();
    assert_eq!(
        jobs::succeed(pool, made_by.id, "w", &doc, Some(diff.id))
            .await
            .unwrap(),
        Some(JobStatus::Succeeded)
    );
    // Another job still uses `shared`.
    let other = job(pool, &shared, &shared).await;

    assert_eq!(diffs::get(pool, diff.id).await.unwrap(), Some(diff.clone()));
    assert!(blobs::in_use(pool, &doc).await.unwrap());
    let err = sqlx::query("DELETE FROM blobs WHERE sha256 = $1")
        .bind(&doc)
        .execute(pool)
        .await;
    assert_eq!(DbError::from(err.unwrap_err()).code(), "missing-reference");

    let mut conn = pool.acquire().await.unwrap();
    let candidates = diffs::blobs_of(&mut conn, diff.id).await.unwrap().unwrap();
    assert_eq!(
        candidates,
        vec![l.clone(), r.clone(), shared.clone(), doc.clone()]
    );
    drop(conn);
    let mut tx = t.db.begin().await.unwrap();
    let mut deleted = diffs::delete(&mut tx, diff.id, &candidates)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
    deleted.sort();
    assert_eq!(deleted, vec![l, r, doc]);
    assert_eq!(diffs::get(pool, diff.id).await.unwrap(), None);
    assert_eq!(jobs::get(pool, made_by.id).await.unwrap(), None);
    assert!(jobs::get(pool, other.id).await.unwrap().is_some());
    assert!(blobs::get(pool, &shared).await.unwrap().is_some());

    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(diffs::blobs_of(&mut conn, diff.id).await.unwrap(), None);
    assert_eq!(
        diffs::delete(&mut conn, diff.id, &candidates)
            .await
            .unwrap(),
        None
    );
    drop(conn);
    t.finish().await;
}

#[tokio::test]
async fn grants_are_per_key_and_keep_blobs_alive() {
    let t = test_db!("grants_are_per_key_and_keep_blobs_alive");
    let pool = t.db.pool();
    let (key_a, _) = api_keys::create(pool, "a", &[]).await.unwrap();
    let (key_b, _) = api_keys::create(pool, "b", &[]).await.unwrap();
    let (x, y, doc) = (
        blob(pool, 0x11).await,
        blob(pool, 0x12).await,
        blob(pool, 0x13).await,
    );

    // A grant is new once per key; another upload by the same key extends it.
    let (first, new) = blobs::grant(pool, &x, Some(key_b.id), Some("text/plain"), later())
        .await
        .unwrap();
    assert!(new);
    let (again, new) = blobs::grant(pool, &x, Some(key_b.id), None, later())
        .await
        .unwrap();
    assert!(!new, "same key, same bytes");
    assert!(again.expires_at >= first.expires_at);
    assert_eq!(again.created_at, first.created_at);
    assert_eq!(again.media_type.as_deref(), Some("text/plain"));
    let (_, new) = blobs::grant(pool, &x, None, None, later()).await.unwrap();
    assert!(new, "no key (authentication off) is an owner of its own");

    // Only a live grant of the asking key counts.
    assert!(
        blobs::granted(pool, &x, Some(key_b.id))
            .await
            .unwrap()
            .is_some()
    );
    assert!(blobs::granted(pool, &x, None).await.unwrap().is_some());
    assert!(
        blobs::granted(pool, &x, Some(key_a.id))
            .await
            .unwrap()
            .is_none()
    );
    blobs::grant(pool, &y, Some(key_a.id), None, earlier())
        .await
        .unwrap();
    assert!(
        blobs::granted(pool, &y, Some(key_a.id))
            .await
            .unwrap()
            .is_none()
    );
    assert!(!blobs::held(pool, &y).await.unwrap(), "expired grant");
    assert!(blobs::held(pool, &x).await.unwrap());
    let (_, new) = blobs::grant(pool, &y, Some(key_a.id), None, later())
        .await
        .unwrap();
    assert!(new, "an expired grant taken up again is new");
    sqlx::query(
        "UPDATE blob_grants SET expires_at = now() - interval '1 second' WHERE sha256 = $1",
    )
    .bind(&y)
    .execute(pool)
    .await
    .unwrap();

    // Key A deletes its diff of X and Y: Y goes (its grant expired), X stays
    // because key B (and the keyless owner) still hold it.
    let diff = saved_diff(pool, &doc, &x, &y).await;
    let mut conn = pool.acquire().await.unwrap();
    let candidates = diffs::blobs_of(&mut conn, diff.id).await.unwrap().unwrap();
    drop(conn);
    let mut tx = t.db.begin().await.unwrap();
    let mut deleted = diffs::delete(&mut tx, diff.id, &candidates)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
    deleted.sort();
    assert_eq!(deleted, vec![y.clone(), doc]);
    assert!(blobs::get(pool, &x).await.unwrap().is_some());
    assert!(
        blobs::granted(pool, &y, Some(key_a.id))
            .await
            .unwrap()
            .is_none()
    );

    // Expired grants are tidied away; live ones stay.
    sqlx::query(
        "UPDATE blob_grants SET expires_at = now() - interval '1 second' WHERE api_key_id IS NULL",
    )
    .execute(pool)
    .await
    .unwrap();
    assert_eq!(retention::expire_grants(pool, 100).await.unwrap(), 1);
    assert!(
        blobs::granted(pool, &x, Some(key_b.id))
            .await
            .unwrap()
            .is_some()
    );
    t.finish().await;
}

#[tokio::test]
async fn delete_locks_exclude_writers_and_other_deleters() {
    let t = test_db!("delete_locks_exclude_writers_and_other_deleters");
    let pool = t.db.pool();
    // Hashes no other test uses: the locks are shared by the whole database.
    let (x, y) = ("a7".repeat(32), "a8".repeat(32));
    let mut deleter = t.db.dedicated().await.unwrap();
    blobs::lock_for_delete(&mut deleter, &[y.clone(), x.clone(), x.clone()])
        .await
        .unwrap();
    let mut other = t.db.dedicated().await.unwrap();
    assert!(
        blobs::try_lock_for_delete(&mut other, &[x.clone(), y.clone()])
            .await
            .unwrap()
            .is_empty()
    );
    // A writer waits for the deleter.
    let writer = {
        let db = t.db.clone();
        let x = x.clone();
        tokio::spawn(async move {
            let mut tx = db.begin().await.unwrap();
            blobs::lock_shared(&mut *tx, &x).await.unwrap();
            tx.commit().await.unwrap();
        })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !writer.is_finished(),
        "the writer waits for the delete lock"
    );
    drop(deleter);
    tokio::time::timeout(Duration::from_secs(10), writer)
        .await
        .expect("closing the deleter's connection releases its locks")
        .unwrap();
    assert_eq!(
        blobs::try_lock_for_delete(&mut other, std::slice::from_ref(&x))
            .await
            .unwrap(),
        vec![x]
    );
    drop(other);
    let _ = pool;
    t.finish().await;
}

#[tokio::test]
async fn retention_sweeps_only_expired_unreferenced_rows() {
    let t = test_db!("retention_sweeps_only_expired_unreferenced_rows");
    let pool = t.db.pool();
    for n in 1..=4 {
        blobs::upsert(pool, &hex(n), 1, None, earlier())
            .await
            .unwrap();
    }
    blobs::upsert(pool, &hex(5), 1, None, later())
        .await
        .unwrap();
    // Blob 1 backs a live job; blobs 2 and 3 back an expired diff; 4 is free.
    let live = job(pool, &hex(1), &hex(1)).await;
    let stats = json!({});
    let new = diffs::NewDiff {
        api_key_id: None,
        document_blob: &hex(2),
        left_blob: Some(&hex(3)),
        right_blob: None,
        status: "identical",
        kind: "binary",
        stats: Some(&stats),
        expires_at: earlier(),
    };
    let old = diffs::create(pool, &new).await.unwrap();
    assert_eq!(
        diffs::get(pool, old.id).await.unwrap(),
        None,
        "expired diffs are hidden"
    );

    let mut tx = t.db.begin().await.unwrap();
    assert_eq!(retention::expire_diffs(&mut tx, 100).await.unwrap(), 1);
    let candidates = retention::blob_sweep_candidates(&mut *tx, 100)
        .await
        .unwrap();
    let mut swept = retention::sweep_blobs(&mut *tx, &candidates).await.unwrap();
    tx.commit().await.unwrap();
    swept.sort();
    assert_eq!(swept, vec![hex(2), hex(3), hex(4)]);
    assert!(blobs::get(pool, &hex(1)).await.unwrap().is_some());
    assert!(blobs::get(pool, &hex(5)).await.unwrap().is_some());

    assert_eq!(retention::expire_jobs(pool, 100).await.unwrap(), 0);
    sqlx::query("UPDATE jobs SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(live.id)
        .execute(pool)
        .await
        .unwrap();
    jobs::claim(pool, "w", Duration::from_secs(30), 1)
        .await
        .unwrap();
    assert_eq!(
        retention::expire_jobs(pool, 100).await.unwrap(),
        0,
        "running jobs stay"
    );
    jobs::fail(pool, live.id, "w", "internal", "x", None)
        .await
        .unwrap();
    assert_eq!(retention::expire_jobs(pool, 100).await.unwrap(), 1);
    let candidates = retention::blob_sweep_candidates(pool, 100).await.unwrap();
    assert_eq!(candidates, vec![hex(1)]);
    // A live grant keeps an expired, unreferenced blob.
    blobs::grant(pool, &hex(1), None, None, later())
        .await
        .unwrap();
    assert!(
        retention::sweep_blobs(pool, &candidates)
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::query("UPDATE blob_grants SET expires_at = now() - interval '1 second'")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        retention::sweep_blobs(pool, &candidates).await.unwrap(),
        vec![hex(1)]
    );
    t.finish().await;
}

#[tokio::test]
async fn usage_and_audit_are_append_only() {
    let t = test_db!("usage_and_audit_are_append_only");
    let pool = t.db.pool();
    let key = Uuid::now_v7();
    let since = Utc::now() - TimeDelta::seconds(5);
    for bytes in [10, 20] {
        let usage = Usage {
            api_key_id: Some(key),
            operation: "diff.sync",
            outcome: "ok",
            bytes_in: bytes,
            bytes_out: 1,
            duration_ms: 3,
        };
        records::record_usage(pool, &usage).await.unwrap();
    }
    let totals = records::usage_totals(pool, key, since).await.unwrap();
    assert_eq!(
        totals,
        records::UsageTotals {
            calls: 2,
            bytes_in: 30,
            bytes_out: 2
        }
    );

    let detail = json!({"blobs": 3});
    for actor in [Actor::Admin, Actor::System, Actor::Key(key)] {
        let event = AuditEvent {
            actor,
            action: "diff.delete",
            target_kind: Some("diff"),
            target_id: Some("0198f0c4-0000-7000-8000-000000000000"),
            detail: &detail,
        };
        records::record_audit(pool, &event).await.unwrap();
    }
    for statement in [
        "UPDATE usage SET bytes_in = 0",
        "DELETE FROM usage",
        "TRUNCATE usage",
        "UPDATE audit SET action = 'x'",
        "DELETE FROM audit",
        "TRUNCATE audit",
    ] {
        let err = sqlx::query(statement).execute(pool).await.unwrap_err();
        assert_eq!(DbError::from(err).code(), "append-only", "{statement}");
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 3);
    t.finish().await;
}
