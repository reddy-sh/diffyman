//! Jobs, which are also the queue.
//!
//! A worker calls [`claim`] to take up to N queued jobs (`FOR UPDATE SKIP
//! LOCKED`, so workers never wait for each other or take the same job). Each
//! claimed job carries a lease: the worker's name and an expiry. The worker
//! renews it with [`heartbeat`], which also reports a cancellation request,
//! and ends the job with [`succeed`], [`fail`] or [`finish_cancelled`]; each
//! of them changes the row only while the worker still holds the lease, and
//! [`succeed`] never marks a job succeeded once a cancel was requested. A
//! sweeper calls [`requeue_expired`] for jobs whose worker stopped renewing.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::{DbError, Result, secs};

/// Where a job is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JobStatus {
    /// Waiting for a worker.
    Queued,
    /// Claimed by a worker that holds its lease.
    Running,
    /// Finished with a result.
    Succeeded,
    /// Finished with an error.
    Failed,
    /// Cancelled before it finished.
    Cancelled,
}

impl JobStatus {
    /// The text stored in the database and shown by the API.
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Succeeded => "succeeded",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }

    /// Whether the job will not change any more.
    pub fn is_final(self) -> bool {
        matches!(
            self,
            JobStatus::Succeeded | JobStatus::Failed | JobStatus::Cancelled
        )
    }
}

impl fmt::Display for JobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for JobStatus {
    type Err = DbError;

    fn from_str(text: &str) -> Result<JobStatus> {
        Ok(match text {
            "queued" => JobStatus::Queued,
            "running" => JobStatus::Running,
            "succeeded" => JobStatus::Succeeded,
            "failed" => JobStatus::Failed,
            "cancelled" => JobStatus::Cancelled,
            _ => return Err(DbError::InvalidValue),
        })
    }
}

/// A job row.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    /// `UUIDv7`, so IDs sort by creation time.
    pub id: Uuid,
    /// The key that created the job.
    pub api_key_id: Option<Uuid>,
    /// Current status.
    pub status: JobStatus,
    /// Diff options as given (JSON object).
    pub options: Value,
    /// Blob ID of the left input.
    pub left_blob: String,
    /// Blob ID of the right input.
    pub right_blob: String,
    /// Name of the left input, used for type detection by extension.
    pub left_name: Option<String>,
    /// Name of the right input.
    pub right_name: Option<String>,
    /// Blob ID of the Diff Document, once succeeded.
    pub result_blob: Option<String>,
    /// The saved diff, once succeeded.
    pub diff_id: Option<Uuid>,
    /// Stable error code of the last failed attempt.
    pub error_code: Option<String>,
    /// Message of the last failed attempt; never file content or paths.
    pub error_message: Option<String>,
    /// Attempts started so far.
    pub attempts: i32,
    /// Attempts allowed.
    pub max_attempts: i32,
    /// Set by a cancel request while running.
    pub cancel_requested: bool,
    /// When the job was created.
    pub created_at: DateTime<Utc>,
    /// Not claimed before this time.
    pub run_after: DateTime<Utc>,
    /// When the first attempt started.
    pub started_at: Option<DateTime<Utc>>,
    /// When the job reached a final status.
    pub finished_at: Option<DateTime<Utc>>,
    /// The worker holding the lease.
    pub lease_owner: Option<String>,
    /// When the lease ends unless renewed.
    pub lease_expires_at: Option<DateTime<Utc>>,
    /// Last heartbeat.
    pub heartbeat_at: Option<DateTime<Utc>>,
    /// When the row may be removed.
    pub expires_at: DateTime<Utc>,
}

/// The row as read, with its status as text.
struct JobRow {
    id: Uuid,
    api_key_id: Option<Uuid>,
    status: String,
    options: Value,
    left_blob: String,
    right_blob: String,
    left_name: Option<String>,
    right_name: Option<String>,
    result_blob: Option<String>,
    diff_id: Option<Uuid>,
    error_code: Option<String>,
    error_message: Option<String>,
    attempts: i32,
    max_attempts: i32,
    cancel_requested: bool,
    created_at: DateTime<Utc>,
    run_after: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    lease_owner: Option<String>,
    lease_expires_at: Option<DateTime<Utc>>,
    heartbeat_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
}

impl TryFrom<JobRow> for Job {
    type Error = DbError;

    fn try_from(row: JobRow) -> Result<Job> {
        Ok(Job {
            id: row.id,
            api_key_id: row.api_key_id,
            status: row.status.parse()?,
            options: row.options,
            left_blob: row.left_blob,
            right_blob: row.right_blob,
            left_name: row.left_name,
            right_name: row.right_name,
            result_blob: row.result_blob,
            diff_id: row.diff_id,
            error_code: row.error_code,
            error_message: row.error_message,
            attempts: row.attempts,
            max_attempts: row.max_attempts,
            cancel_requested: row.cancel_requested,
            created_at: row.created_at,
            run_after: row.run_after,
            started_at: row.started_at,
            finished_at: row.finished_at,
            lease_owner: row.lease_owner,
            lease_expires_at: row.lease_expires_at,
            heartbeat_at: row.heartbeat_at,
            expires_at: row.expires_at,
        })
    }
}

/// A job to create.
#[derive(Clone, Debug)]
pub struct NewJob<'a> {
    /// The key creating it.
    pub api_key_id: Option<Uuid>,
    /// Diff options (a JSON object).
    pub options: &'a Value,
    /// Left input blob ID; the blob row must exist.
    pub left_blob: &'a str,
    /// Right input blob ID; the blob row must exist.
    pub right_blob: &'a str,
    /// Left input name.
    pub left_name: Option<&'a str>,
    /// Right input name.
    pub right_name: Option<&'a str>,
    /// Attempts allowed (1 to 100).
    pub max_attempts: i32,
    /// When the row may be removed.
    pub expires_at: DateTime<Utc>,
}

/// Creates a queued job. Fails with [`DbError::MissingReference`] when an
/// input blob row does not exist.
pub async fn create(executor: impl PgExecutor<'_>, new: &NewJob<'_>) -> Result<Job> {
    sqlx::query_as!(
        JobRow,
        "INSERT INTO jobs (api_key_id, options, left_blob, right_blob, left_name, right_name,
                           max_attempts, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING *",
        new.api_key_id,
        new.options,
        new.left_blob,
        new.right_blob,
        new.left_name,
        new.right_name,
        new.max_attempts,
        new.expires_at,
    )
    .fetch_one(executor)
    .await?
    .try_into()
}

/// The job, or `None`.
pub async fn get(executor: impl PgExecutor<'_>, id: Uuid) -> Result<Option<Job>> {
    sqlx::query_as!(JobRow, "SELECT * FROM jobs WHERE id = $1", id)
        .fetch_optional(executor)
        .await?
        .map(Job::try_from)
        .transpose()
}

/// Claims up to `limit` queued jobs whose `run_after` has passed, oldest
/// first, for `owner`, with a lease of `lease`. Jobs locked by another
/// worker's claim are skipped, never waited for.
pub async fn claim(
    executor: impl PgExecutor<'_>,
    owner: &str,
    lease: Duration,
    limit: i64,
) -> Result<Vec<Job>> {
    let rows = sqlx::query_as!(
        JobRow,
        "WITH next AS (
             SELECT id FROM jobs
             WHERE status = 'queued' AND run_after <= now()
             ORDER BY run_after, id
             LIMIT $1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE jobs AS j
         SET status = 'running',
             attempts = j.attempts + 1,
             lease_owner = $2,
             lease_expires_at = now() + make_interval(secs => $3),
             heartbeat_at = now(),
             started_at = COALESCE(j.started_at, now())
         FROM next
         WHERE j.id = next.id
         RETURNING j.*",
        limit,
        owner,
        secs(lease),
    )
    .fetch_all(executor)
    .await?;
    let mut jobs = rows
        .into_iter()
        .map(Job::try_from)
        .collect::<Result<Vec<_>>>()?;
    jobs.sort_by_key(|job| (job.run_after, job.id));
    Ok(jobs)
}

/// What a heartbeat tells the worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heartbeat {
    /// The lease is renewed; keep going.
    Continue,
    /// The lease is renewed, but a cancel was requested: stop and call
    /// [`finish_cancelled`].
    Cancel,
    /// The worker no longer holds the lease: stop without writing a result.
    Lost,
}

/// Renews the lease for another `lease`.
pub async fn heartbeat(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    owner: &str,
    lease: Duration,
) -> Result<Heartbeat> {
    let cancel = sqlx::query_scalar!(
        "UPDATE jobs
         SET heartbeat_at = now(), lease_expires_at = now() + make_interval(secs => $3)
         WHERE id = $1 AND status = 'running' AND lease_owner = $2
         RETURNING cancel_requested",
        id,
        owner,
        secs(lease),
    )
    .fetch_optional(executor)
    .await?;
    Ok(match cancel {
        None => Heartbeat::Lost,
        Some(true) => Heartbeat::Cancel,
        Some(false) => Heartbeat::Continue,
    })
}

/// Ends the job with its result: `succeeded`, unless a cancel was requested
/// while it ran, in which case it ends `cancelled` with no result (a cancel
/// always wins over a result that arrives after it). Returns the new status,
/// or `None` when `owner` no longer holds the lease.
///
/// Called in the transaction that saves the result, which the caller rolls
/// back on `Cancelled` so that the result is discarded.
pub async fn succeed(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    owner: &str,
    result_blob: &str,
    diff_id: Option<Uuid>,
) -> Result<Option<JobStatus>> {
    let status = sqlx::query_scalar!(
        "UPDATE jobs
         SET status = CASE WHEN cancel_requested THEN 'cancelled' ELSE 'succeeded' END,
             result_blob = CASE WHEN cancel_requested THEN NULL ELSE $3::text END,
             diff_id = CASE WHEN cancel_requested THEN NULL ELSE $4::uuid END,
             finished_at = now(), lease_owner = NULL, lease_expires_at = NULL
         WHERE id = $1 AND status = 'running' AND lease_owner = $2
         RETURNING status",
        id,
        owner,
        result_blob,
        diff_id,
    )
    .fetch_optional(executor)
    .await?;
    status.map(|s| s.parse()).transpose()
}

/// Records a failed attempt. With `retry_after`, the job is queued again
/// after that delay while attempts remain; otherwise, or after the last
/// attempt, it is failed. Returns the new status, or `None` when `owner` no
/// longer holds the lease.
pub async fn fail(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    owner: &str,
    code: &str,
    message: &str,
    retry_after: Option<Duration>,
) -> Result<Option<JobStatus>> {
    let status = sqlx::query_scalar!(
        "UPDATE jobs
         SET status = CASE WHEN $5::float8 IS NOT NULL AND attempts < max_attempts
                           THEN 'queued' ELSE 'failed' END,
             run_after = CASE WHEN $5::float8 IS NOT NULL AND attempts < max_attempts
                              THEN now() + make_interval(secs => $5::float8) ELSE run_after END,
             finished_at = CASE WHEN $5::float8 IS NOT NULL AND attempts < max_attempts
                                THEN NULL ELSE now() END,
             error_code = $3,
             error_message = $4,
             lease_owner = NULL,
             lease_expires_at = NULL
         WHERE id = $1 AND status = 'running' AND lease_owner = $2
         RETURNING status",
        id,
        owner,
        code,
        message,
        retry_after.map(secs),
    )
    .fetch_optional(executor)
    .await?;
    status.map(|s| s.parse()).transpose()
}

/// Ends a running job whose cancellation was requested. Returns `false` when
/// `owner` no longer holds the lease.
pub async fn finish_cancelled(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    owner: &str,
) -> Result<bool> {
    let done = sqlx::query!(
        "UPDATE jobs
         SET status = 'cancelled', finished_at = now(), lease_owner = NULL, lease_expires_at = NULL
         WHERE id = $1 AND status = 'running' AND lease_owner = $2",
        id,
        owner,
    )
    .execute(executor)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Requests cancellation. A queued job is cancelled at once; a running job is
/// flagged, and its worker stops at its next heartbeat; a finished job is
/// unchanged. Returns the status after the request, or `None` for an unknown
/// job.
pub async fn cancel(executor: impl PgExecutor<'_>, id: Uuid) -> Result<Option<JobStatus>> {
    let status = sqlx::query_scalar!(
        "UPDATE jobs
         SET status = CASE WHEN status = 'queued' THEN 'cancelled' ELSE status END,
             finished_at = CASE WHEN status = 'queued' THEN now() ELSE finished_at END,
             cancel_requested = cancel_requested OR status IN ('queued', 'running')
         WHERE id = $1
         RETURNING status",
        id,
    )
    .fetch_optional(executor)
    .await?;
    status.map(|s| s.parse()).transpose()
}

/// What [`requeue_expired`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequeueReport {
    /// Jobs queued again for another attempt.
    pub requeued: u64,
    /// Jobs failed with `lease-expired` after their last attempt.
    pub failed: u64,
    /// Jobs cancelled because a cancel was requested.
    pub cancelled: u64,
}

/// Handles up to `limit` running jobs whose lease has ended: queued again
/// while attempts remain, `failed` with code `lease-expired` after the last
/// one, or `cancelled` when a cancel was requested.
pub async fn requeue_expired(executor: impl PgExecutor<'_>, limit: i64) -> Result<RequeueReport> {
    let statuses = sqlx::query_scalar!(
        "WITH expired AS (
             SELECT id FROM jobs
             WHERE status = 'running' AND lease_expires_at < now()
             ORDER BY lease_expires_at
             LIMIT $1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE jobs AS j
         SET status = CASE WHEN j.cancel_requested THEN 'cancelled'
                           WHEN j.attempts < j.max_attempts THEN 'queued'
                           ELSE 'failed' END,
             error_code = CASE WHEN NOT j.cancel_requested AND j.attempts >= j.max_attempts
                               THEN 'lease-expired' ELSE j.error_code END,
             error_message = CASE WHEN NOT j.cancel_requested AND j.attempts >= j.max_attempts
                                  THEN 'the worker stopped renewing its lease'
                                  ELSE j.error_message END,
             finished_at = CASE WHEN j.cancel_requested OR j.attempts >= j.max_attempts
                                THEN now() ELSE NULL END,
             run_after = now(),
             lease_owner = NULL,
             lease_expires_at = NULL
         FROM expired
         WHERE j.id = expired.id
         RETURNING j.status",
        limit,
    )
    .fetch_all(executor)
    .await?;
    let mut report = RequeueReport::default();
    for status in statuses {
        match status.parse()? {
            JobStatus::Queued => report.requeued += 1,
            JobStatus::Failed => report.failed += 1,
            _ => report.cancelled += 1,
        }
    }
    Ok(report)
}
