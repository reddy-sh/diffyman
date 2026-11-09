//! Retention: the sweeper removes expired content.
//!
//! Every `sweep_interval` (and once at start) it:
//!
//! 1. handles running jobs whose lease expired (as the worker also does);
//! 2. deletes expired saved diffs and the jobs that produced them;
//! 3. deletes expired jobs that are not running;
//! 4. deletes expired blob grants;
//! 5. deletes expired blobs that nothing holds (no job or saved diff
//!    references them, no key's grant is live): the rows are deleted and
//!    committed under each blob's delete lock, then the objects (an object
//!    that cannot be deleted is left without a row, for step 6);
//! 6. deletes objects without a row (a failed upload or deletion) and
//!    unfinished streamed uploads under `staging/`, once older than
//!    `orphan_grace`.
//!
//! Each step works in batches and skips rows locked by someone else, so
//! several servers can sweep one database.

use chrono::{DateTime, Utc};
use diffyman_db::records::{self, Actor, AuditEvent};
use diffyman_db::{Acquire, DbError, blobs, jobs, retention};
use diffyman_storage::{Area, BlobId, ObjectKind};
use futures::StreamExt;
use serde_json::json;
use tokio::sync::watch;

use crate::content::delete_objects;
use crate::error::ApiError;
use crate::state::AppState;

/// Rows per batch.
const BATCH: i64 = 500;

/// What one sweep removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Expired leases queued again.
    pub requeued: u64,
    /// Expired leases failed after their last attempt or cancelled.
    pub failed: u64,
    /// Saved diffs.
    pub diffs: u64,
    /// Jobs.
    pub jobs: u64,
    /// Expired blob grants.
    pub grants: u64,
    /// Blob rows and their objects.
    pub blobs: u64,
    /// Objects without a row.
    pub orphans: u64,
    /// Unfinished streamed uploads.
    pub staging: u64,
}

impl SweepReport {
    fn removed_anything(&self) -> bool {
        *self != SweepReport::default()
    }
}

/// Runs a sweep every `sweep_interval` until `shutdown` turns `true`.
pub async fn run(state: AppState, mut shutdown: watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(state.config.retention.sweep_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown.changed() => {}
        }
        if *shutdown.borrow() {
            return;
        }
        match sweep_once(&state).await {
            Ok(report) if report.removed_anything() => tracing::info!(
                requeued = report.requeued,
                failed = report.failed,
                diffs = report.diffs,
                jobs = report.jobs,
                grants = report.grants,
                blobs = report.blobs,
                orphans = report.orphans,
                staging = report.staging,
                "sweep finished"
            ),
            Ok(_) => tracing::debug!("sweep found nothing to remove"),
            Err(error) => tracing::warn!(code = error.code, "sweep failed"),
        }
    }
}

/// One full sweep.
pub async fn sweep_once(state: &AppState) -> Result<SweepReport, ApiError> {
    let db = &state.db;
    let mut report = SweepReport::default();
    let leases = jobs::requeue_expired(db.pool(), BATCH).await?;
    report.requeued = leases.requeued;
    report.failed = leases.failed + leases.cancelled;
    loop {
        let mut tx = db.begin().await?;
        let n = retention::expire_diffs(&mut tx, BATCH).await?;
        tx.commit().await.map_err(DbError::from)?;
        report.diffs += n;
        if n < BATCH.unsigned_abs() {
            break;
        }
    }
    loop {
        let n = retention::expire_jobs(db.pool(), BATCH).await?;
        report.jobs += n;
        if n < BATCH.unsigned_abs() {
            break;
        }
    }
    loop {
        let n = retention::expire_grants(db.pool(), BATCH).await?;
        report.grants += n;
        if n < BATCH.unsigned_abs() {
            break;
        }
    }
    loop {
        // Session locks: this connection is closed, never pooled, when dropped.
        let mut conn = db.dedicated().await?;
        let candidates = retention::blob_sweep_candidates(&mut *conn, BATCH).await?;
        if candidates.is_empty() {
            break;
        }
        let locked = blobs::try_lock_for_delete(&mut conn, &candidates).await?;
        let mut tx = (&mut *conn).begin().await.map_err(DbError::from)?;
        let ids = retention::sweep_blobs(&mut *tx, &locked).await?;
        tx.commit().await.map_err(DbError::from)?;
        delete_objects(state, &ids).await;
        drop(conn);
        report.blobs += ids.len() as u64;
        if ids.is_empty() || (candidates.len() as u64) < BATCH.unsigned_abs() {
            break;
        }
    }
    let grace = chrono::TimeDelta::from_std(state.config.retention.orphan_grace)
        .unwrap_or(chrono::TimeDelta::days(1));
    let (orphans, staging) = sweep_orphans(state, Utc::now() - grace).await?;
    report.orphans = orphans;
    report.staging = staging;
    if let Some(limiter) = &state.limiter {
        limiter.retain_recent();
    }
    if report.removed_anything() {
        let detail = json!({
            "diffs": report.diffs,
            "jobs": report.jobs,
            "grants": report.grants,
            "blobs": report.blobs,
            "orphans": report.orphans,
            "staging": report.staging,
            "requeued": report.requeued,
            "failed": report.failed,
        });
        records::record_audit(
            db.pool(),
            &AuditEvent {
                actor: Actor::System,
                action: "retention.sweep",
                target_kind: None,
                target_id: None,
                detail: &detail,
            },
        )
        .await?;
    }
    Ok(report)
}

/// Deletes objects written before `older_than` that have no row, and
/// unfinished streamed uploads started before it. Returns both counts.
pub async fn sweep_orphans(
    state: &AppState,
    older_than: DateTime<Utc>,
) -> Result<(u64, u64), ApiError> {
    let store = &state.store;
    let mut candidates: Vec<BlobId> = Vec::new();
    let mut orphans = 0;
    {
        let mut listing = store.list(Area::Blobs);
        while let Some(object) = listing.next().await {
            let object = object?;
            if let ObjectKind::Blob(id) = object.kind
                && object.last_modified < older_than
            {
                candidates.push(id);
            }
        }
    }
    for batch in candidates.chunks(usize::try_from(BATCH).unwrap_or(500)) {
        let hexes: Vec<String> = batch.iter().map(BlobId::to_hex).collect();
        let existing = blobs::existing(state.db.pool(), &hexes).await?;
        for id in batch {
            if !existing.contains(&id.to_hex()) {
                store.delete(*id).await?;
                orphans += 1;
            }
        }
    }
    let mut stale = Vec::new();
    {
        let mut listing = store.list(Area::Staging);
        while let Some(object) = listing.next().await {
            let object = object?;
            if object.last_modified < older_than {
                stale.push(object.kind);
            }
        }
    }
    let mut staging = 0;
    for kind in &stale {
        if store.delete_object(kind).await.is_ok() {
            staging += 1;
        }
    }
    Ok((orphans, staging))
}
