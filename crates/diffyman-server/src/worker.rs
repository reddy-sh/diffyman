//! The job worker.
//!
//! One loop per process claims queued jobs (`FOR UPDATE SKIP LOCKED`, so
//! several servers on one database share the queue) whenever one of its
//! `max_parallel_jobs` slots is free, and runs each claimed job in its own
//! task:
//!
//! 1. A first heartbeat confirms the lease and picks up a cancel that
//!    arrived since the claim; later ones renew it every third of the lease.
//! 2. The inputs are read from the blob store, refused when together they
//!    are over the job memory budget (`job_memory_bytes / 4`).
//! 3. The engine runs on a blocking thread. It cannot be interrupted, so a
//!    job that runs past its time budget, or is cancelled, is ended in the
//!    database at once while its thread keeps its slot until the engine
//!    returns: the number of engine threads never exceeds the slots.
//! 4. The document is stored, and the saved diff and the job's success are
//!    written in one transaction that only commits while the lease is held.
//!    A cancel requested while the engine ran wins: the transaction is
//!    rolled back, so no saved diff is left, and the job ends `cancelled`.
//!
//! A job keeps its slot until both its task (steps 1 to 4, storing and
//! committing included) and its engine thread are done, so no more than
//! `max_parallel_jobs` jobs hold memory at once.
//!
//! Only infrastructure errors (database or blob store unavailable) are
//! retried, after `retry_delay` times the attempt number; errors of the
//! inputs or options fail the job at once. Jobs whose worker stopped
//! renewing its lease are queued again (or failed after their last attempt)
//! by any worker, every half lease.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use diffyman_db::diffs::{self, NewDiff};
use diffyman_db::jobs::{self, Heartbeat, Job, JobStatus, RequeueReport};
use diffyman_storage::BlobId;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::content::{expiry, read_blob, store_bytes};
use crate::engine::{self, DiffInputs, Summary};
use crate::error::ApiError;
use crate::state::AppState;

/// Longest stored error message.
const MAX_MESSAGE_CHARS: usize = 2000;

/// Why an attempt did not succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// Stable error code.
    pub code: &'static str,
    /// Message for the job record; never file content or paths.
    pub message: String,
    /// Whether another attempt may succeed (infrastructure errors only).
    pub retry: bool,
    /// The lease was lost: write nothing.
    pub lost: bool,
}

impl Failure {
    fn lost() -> Failure {
        Failure {
            code: "lease-lost",
            message: String::new(),
            retry: false,
            lost: true,
        }
    }
}

impl From<ApiError> for Failure {
    fn from(error: ApiError) -> Failure {
        Failure {
            code: error.code,
            retry: error.status == axum::http::StatusCode::SERVICE_UNAVAILABLE,
            message: error.message,
            lost: false,
        }
    }
}

impl From<diffyman_db::DbError> for Failure {
    fn from(error: diffyman_db::DbError) -> Failure {
        ApiError::from(error).into()
    }
}

/// How a job task ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Succeeded with this saved diff.
    Succeeded(Uuid),
    /// The attempt failed.
    Failed(Failure),
    /// A cancel was requested.
    Cancelled,
    /// Another worker owns the job now.
    Lost,
    /// The time budget ran out.
    TimedOut,
    /// The server is stopping.
    Interrupted,
}

/// A slot that wakes the claim loop when released.
struct Slot {
    _permit: OwnedSemaphorePermit,
    state: AppState,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.state.jobs_notify.notify_one();
    }
}

/// Runs the claim loop until `shutdown` turns `true`, then lets running
/// jobs finish for the shutdown grace period and hands the rest back to the
/// queue.
pub async fn run(state: AppState, mut shutdown: watch::Receiver<bool>) {
    let parallel = usize::try_from(state.config.limits.max_parallel_jobs).unwrap_or(1);
    let slots = Arc::new(Semaphore::new(parallel));
    let (interrupt, interrupted) = watch::channel(false);
    let mut tasks = JoinSet::new();
    let lease = state.config.limits.job_lease;
    let poll = state.tuning.poll_interval;
    let mut next_requeue = Instant::now();
    let mut backoff = Duration::ZERO;
    tracing::info!(
        worker = state.worker_id.as_str(),
        parallel_jobs = parallel,
        "worker started"
    );
    loop {
        if *shutdown.borrow() {
            break;
        }
        while tasks.try_join_next().is_some() {}
        if Instant::now() >= next_requeue {
            requeue(&state).await;
            next_requeue = Instant::now() + lease / 2;
        }
        let free = slots.available_permits();
        let mut full_batch = false;
        if free > 0 {
            let limit = i64::try_from(free).unwrap_or(1);
            match jobs::claim(state.db.pool(), &state.worker_id, lease, limit).await {
                Ok(claimed) => {
                    backoff = Duration::ZERO;
                    full_batch = claimed.len() == free;
                    for job in claimed {
                        let Ok(permit) = slots.clone().try_acquire_owned() else {
                            hand_back(&state, &job).await;
                            continue;
                        };
                        let slot = Slot {
                            _permit: permit,
                            state: state.clone(),
                        };
                        tasks.spawn(run_job(state.clone(), job, slot, interrupted.clone()));
                    }
                }
                Err(error) => {
                    tracing::warn!(code = error.code(), "claiming jobs failed");
                    backoff = (backoff * 2).clamp(poll, Duration::from_secs(30));
                }
            }
        }
        if full_batch {
            continue;
        }
        let until_requeue = next_requeue.saturating_duration_since(Instant::now());
        let wait = poll
            .max(backoff)
            .min(until_requeue.max(Duration::from_millis(10)));
        tokio::select! {
            () = state.jobs_notify.notified() => {}
            () = tokio::time::sleep(wait) => {}
            _ = shutdown.changed() => {}
        }
    }
    let drain = async { while tasks.join_next().await.is_some() {} };
    if tokio::time::timeout(state.tuning.shutdown_grace, drain)
        .await
        .is_err()
    {
        let _ = interrupt.send(true);
        let drain = async { while tasks.join_next().await.is_some() {} };
        let _ = tokio::time::timeout(Duration::from_secs(5), drain).await;
        tasks.abort_all();
    }
    tracing::info!(worker = state.worker_id.as_str(), "worker stopped");
}

async fn requeue(state: &AppState) {
    match jobs::requeue_expired(state.db.pool(), 100).await {
        Ok(report) if report == RequeueReport::default() => {}
        Ok(report) => tracing::info!(
            requeued = report.requeued,
            failed = report.failed,
            cancelled = report.cancelled,
            "expired leases handled"
        ),
        Err(error) => tracing::warn!(code = error.code(), "requeue of expired leases failed"),
    }
}

/// Gives a claimed job back to the queue without counting against it more
/// than the attempt it was claimed with.
async fn hand_back(state: &AppState, job: &Job) {
    let result = jobs::fail(
        state.db.pool(),
        job.id,
        &state.worker_id,
        "interrupted",
        "the job was handed back to the queue",
        Some(Duration::ZERO),
    )
    .await;
    if let Err(error) = result {
        tracing::warn!(job_id = %job.id, code = error.code(), "handing a job back failed");
    }
}

/// Runs one claimed job and records how it ended.
async fn run_job(
    state: AppState,
    job: Job,
    slot: impl Send + Sync + 'static,
    mut interrupted: watch::Receiver<bool>,
) {
    let slot = Arc::new(slot);
    let started = Instant::now();
    tracing::info!(job_id = %job.id, attempt = job.attempts, "job started");
    let budget = state.config.limits.job_timeout;
    let interrupt = async {
        if interrupted.wait_for(|stop| *stop).await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    let outcome = if let Some(outcome) = first_heartbeat(&state, job.id).await {
        outcome
    } else {
        tokio::select! {
            result = execute(&state, &job, slot) => match result {
                Ok(outcome) => outcome,
                Err(failure) if failure.lost => Outcome::Lost,
                Err(failure) => Outcome::Failed(failure),
            },
            outcome = heartbeats(&state, job.id) => outcome,
            () = tokio::time::sleep(budget) => Outcome::TimedOut,
            () = interrupt => Outcome::Interrupted,
        }
    };
    finish(&state, &job, outcome, started).await;
}

/// Runs one claimed job: the entry point of [`run`]'s tasks, public for
/// tests that claim a job themselves.
pub async fn run_claimed(state: AppState, job: Job) {
    run_claimed_with_slot(state, job, ()).await;
}

/// Like [`run_claimed`], holding `slot` as [`run`] holds a job slot: it is
/// dropped once the job is finished, its result stored and committed, and
/// its engine thread has returned.
pub async fn run_claimed_with_slot(state: AppState, job: Job, slot: impl Send + Sync + 'static) {
    let (_keep, interrupted) = watch::channel(false);
    run_job(state, job, slot, interrupted).await;
}

/// `Some` when the job must not start: cancelled since the claim, or the
/// lease is already gone.
async fn first_heartbeat(state: &AppState, id: Uuid) -> Option<Outcome> {
    let lease = state.config.limits.job_lease;
    match jobs::heartbeat(state.db.pool(), id, &state.worker_id, lease).await {
        Ok(Heartbeat::Continue) | Err(_) => None,
        Ok(Heartbeat::Cancel) => Some(Outcome::Cancelled),
        Ok(Heartbeat::Lost) => Some(Outcome::Lost),
    }
}

/// Renews the lease until a cancel is requested or the lease is lost. A
/// database that stays unreachable for a whole lease counts as lost: another
/// worker may own the job by then.
async fn heartbeats(state: &AppState, id: Uuid) -> Outcome {
    let lease = state.config.limits.job_lease;
    let mut last_ok = Instant::now();
    loop {
        tokio::time::sleep(lease / 3).await;
        match jobs::heartbeat(state.db.pool(), id, &state.worker_id, lease).await {
            Ok(Heartbeat::Continue) => last_ok = Instant::now(),
            Ok(Heartbeat::Cancel) => return Outcome::Cancelled,
            Ok(Heartbeat::Lost) => return Outcome::Lost,
            Err(error) => {
                tracing::warn!(job_id = %id, code = error.code(), "heartbeat failed");
                if last_ok.elapsed() >= lease {
                    return Outcome::Lost;
                }
            }
        }
    }
}

fn blob_id(text: &str) -> Result<BlobId, Failure> {
    text.parse()
        .map_err(|_| Failure::from(ApiError::internal()))
}

/// Runs the job while holding `slot`; the engine thread holds a second
/// reference, so the slot is free only when both are done. Ends with
/// [`Outcome::Succeeded`], or [`Outcome::Cancelled`] when a cancel was
/// requested before the result was committed.
async fn execute<S: Send + Sync + 'static>(
    state: &AppState,
    job: &Job,
    slot: Arc<S>,
) -> Result<Outcome, Failure> {
    let limit = state.config.limits.max_job_input_bytes();
    let left = read_blob(state, blob_id(&job.left_blob)?, limit).await?;
    let rest = limit.saturating_sub(left.len() as u64);
    let right = read_blob(state, blob_id(&job.right_blob)?, rest).await?;
    let inputs = DiffInputs {
        left,
        right,
        left_name: job.left_name.clone(),
        right_name: job.right_name.clone(),
        options: serde_json::to_string(&job.options).map_err(|_| ApiError::internal())?,
    };
    let document = engine::run(inputs, Arc::clone(&slot))
        .await?
        .map_err(|e| Failure {
            retry: false,
            ..Failure::from(ApiError::from(e))
        })?;
    let summary = Summary::of(&document)?;
    let retention = state.config.retention;
    let blob_expiry = expiry(retention.blob_ttl.max(retention.diff_ttl));
    let size = document.len();
    let stored = store_bytes(
        state,
        Bytes::from(document),
        Some("application/json"),
        blob_expiry,
    )
    .await?;
    let document_blob = stored.id.to_hex();
    let mut tx = state.db.begin().await?;
    let diff = diffs::create(
        &mut *tx,
        &NewDiff {
            api_key_id: job.api_key_id,
            document_blob: &document_blob,
            left_blob: Some(&job.left_blob),
            right_blob: Some(&job.right_blob),
            status: &summary.status,
            kind: &summary.kind,
            stats: summary.stats.as_ref(),
            expires_at: expiry(retention.diff_ttl),
        },
    )
    .await?;
    let status = jobs::succeed(
        &mut *tx,
        job.id,
        &state.worker_id,
        &document_blob,
        Some(diff.id),
    )
    .await?;
    match status {
        Some(JobStatus::Succeeded) => {}
        Some(_) => {
            // A cancel arrived while the engine ran: the saved diff goes with
            // the rollback, and the job is ended `cancelled` (the stored
            // document has no reference left; the sweep removes it).
            let _ = tx.rollback().await;
            return Ok(Outcome::Cancelled);
        }
        None => {
            let _ = tx.rollback().await;
            return Err(Failure::lost());
        }
    }
    tx.commit().await.map_err(diffyman_db::DbError::from)?;
    tracing::debug!(job_id = %job.id, size, "job result stored");
    drop(slot);
    Ok(Outcome::Succeeded(diff.id))
}

fn truncate(message: &str) -> String {
    message.chars().take(MAX_MESSAGE_CHARS).collect()
}

async fn finish(state: &AppState, job: &Job, outcome: Outcome, started: Instant) {
    let pool = state.db.pool();
    let owner = state.worker_id.as_str();
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let fail = |code: &'static str, message: String, retry: Option<Duration>| async move {
        match jobs::fail(pool, job.id, owner, code, &truncate(&message), retry).await {
            Ok(Some(status)) => status.as_str(),
            Ok(None) => "lost",
            Err(error) => {
                tracing::warn!(job_id = %job.id, code = error.code(), "recording a job failure failed");
                "unrecorded"
            }
        }
    };
    let (outcome_name, code) = match outcome {
        Outcome::Succeeded(diff_id) => {
            tracing::info!(job_id = %job.id, diff_id = %diff_id, duration_ms, outcome = "succeeded", "job finished");
            return;
        }
        Outcome::Failed(failure) => {
            let retry = failure.retry.then(|| {
                state.tuning.retry_delay * u32::try_from(job.attempts.max(1)).unwrap_or(1)
            });
            (
                fail(failure.code, failure.message, retry).await,
                failure.code,
            )
        }
        Outcome::Cancelled => {
            let name = match jobs::finish_cancelled(pool, job.id, owner).await {
                Ok(true) => "cancelled",
                Ok(false) => "lost",
                Err(_) => "unrecorded",
            };
            (name, "cancelled")
        }
        Outcome::Lost => ("lost", "lease-lost"),
        Outcome::TimedOut => {
            let message = format!(
                "the job ran longer than its time budget of {} s",
                state.config.limits.job_timeout.as_secs()
            );
            (fail("job-timeout", message, None).await, "job-timeout")
        }
        Outcome::Interrupted => {
            let message = "the server stopped while the job ran".to_owned();
            (
                fail("interrupted", message, Some(Duration::ZERO)).await,
                "interrupted",
            )
        }
    };
    tracing::info!(
        job_id = %job.id,
        duration_ms,
        outcome = outcome_name,
        code,
        "job finished"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_unavailable_dependencies_are_retried() {
        let retry: Failure = ApiError::unavailable("db-unavailable", "x", 1).into();
        assert!(retry.retry);
        let no: Failure = ApiError::too_large("job-too-large", "x").into();
        assert!(!no.retry);
        let no: Failure = ApiError::internal().into();
        assert!(!no.retry);
        assert_eq!(truncate(&"é".repeat(3000)).chars().count(), 2000);
    }
}
