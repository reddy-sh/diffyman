//! A cancel requested while a job runs wins over the result that arrives
//! after it (`docs/api/README.md`, `POST /v1/jobs/{id}/cancel`). Runs against
//! the test database and skips when none is reachable; see `common/mod.rs`.

#[allow(dead_code)]
mod common;

use std::time::Duration;

use chrono::{TimeDelta, Utc};
use diffyman_db::jobs::{self, JobStatus, NewJob};
use diffyman_db::{PgPool, blobs};
use serde_json::json;

macro_rules! test_db {
    ($name:literal) => {
        match common::test_db($name).await {
            Some(t) => t,
            None => return,
        }
    };
}

async fn blob(pool: &PgPool, n: u8) -> String {
    let id = format!("{n:02x}").repeat(32);
    blobs::upsert(pool, &id, 4, None, Utc::now() + TimeDelta::days(1))
        .await
        .unwrap();
    id
}

async fn running_job(pool: &PgPool) -> uuid::Uuid {
    let (left, right) = (blob(pool, 1).await, blob(pool, 2).await);
    let options = json!({});
    let job = jobs::create(
        pool,
        &NewJob {
            api_key_id: None,
            options: &options,
            left_blob: &left,
            right_blob: &right,
            left_name: None,
            right_name: None,
            max_attempts: 3,
            expires_at: Utc::now() + TimeDelta::days(1),
        },
    )
    .await
    .unwrap();
    let claimed = jobs::claim(pool, "w", Duration::from_secs(30), 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    job.id
}

#[tokio::test]
async fn test_cancel_requested_while_running_ends_the_job_cancelled() {
    let t = test_db!("test_cancel_requested_while_running_ends_the_job_cancelled");
    let pool = t.db.pool();
    let id = running_job(pool).await;
    assert_eq!(
        jobs::cancel(pool, id).await.unwrap(),
        Some(JobStatus::Running)
    );

    // The engine finished after the cancel: its result is not recorded.
    let document = blob(pool, 3).await;
    assert_eq!(
        jobs::succeed(pool, id, "w", &document, None).await.unwrap(),
        Some(JobStatus::Cancelled)
    );
    let job = jobs::get(pool, id).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::Cancelled);
    assert_eq!((job.result_blob, job.diff_id), (None, None));
    assert!(job.finished_at.is_some() && job.lease_owner.is_none());
    t.finish().await;
}

#[tokio::test]
async fn test_success_without_a_cancel_is_recorded() {
    let t = test_db!("test_success_without_a_cancel_is_recorded");
    let pool = t.db.pool();
    let id = running_job(pool).await;
    let document = blob(pool, 3).await;
    assert_eq!(
        jobs::succeed(pool, id, "w", &document, None).await.unwrap(),
        Some(JobStatus::Succeeded)
    );
    let job = jobs::get(pool, id).await.unwrap().unwrap();
    assert_eq!(job.result_blob.as_deref(), Some(document.as_str()));
    // Too late: a finished job stays as it is.
    assert_eq!(
        jobs::cancel(pool, id).await.unwrap(),
        Some(JobStatus::Succeeded)
    );
    t.finish().await;
}
