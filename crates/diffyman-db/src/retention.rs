//! Retention: expired rows are removed in batches by the sweeper.
//!
//! Each function handles at most `limit` rows and skips rows locked by
//! someone else, so several sweepers (or a sweeper and the API) never block
//! each other. Blobs go in three steps, as for a deleted diff (see
//! [`crate::blobs`]): pick [`blob_sweep_candidates`], take
//! [`crate::blobs::try_lock_for_delete`] on them, run [`sweep_blobs`] on the
//! locked ones in a transaction and commit, then delete the returned objects
//! from the blob store.

use sqlx::PgExecutor;

use crate::{PgConnection, Result};

/// Deletes up to `limit` expired saved diffs and the jobs that produced
/// them. Their blobs are left to [`sweep_blobs`].
pub async fn expire_diffs(conn: &mut PgConnection, limit: i64) -> Result<u64> {
    let ids = sqlx::query_scalar!(
        "SELECT id FROM diffs WHERE expires_at <= now()
         ORDER BY expires_at LIMIT $1 FOR UPDATE SKIP LOCKED",
        limit,
    )
    .fetch_all(&mut *conn)
    .await?;
    if ids.is_empty() {
        return Ok(0);
    }
    sqlx::query!("DELETE FROM jobs WHERE diff_id = ANY($1)", &ids)
        .execute(&mut *conn)
        .await?;
    let done = sqlx::query!("DELETE FROM diffs WHERE id = ANY($1)", &ids)
        .execute(&mut *conn)
        .await?;
    Ok(done.rows_affected())
}

/// Deletes up to `limit` expired jobs that are not running.
pub async fn expire_jobs(executor: impl PgExecutor<'_>, limit: i64) -> Result<u64> {
    let done = sqlx::query!(
        "DELETE FROM jobs WHERE id IN (
             SELECT id FROM jobs WHERE expires_at <= now() AND status <> 'running'
             ORDER BY expires_at LIMIT $1 FOR UPDATE SKIP LOCKED
         )",
        limit,
    )
    .execute(executor)
    .await?;
    Ok(done.rows_affected())
}

/// Deletes up to `limit` expired blob grants. An expired grant no longer
/// holds its blob; this only tidies the table.
pub async fn expire_grants(executor: impl PgExecutor<'_>, limit: i64) -> Result<u64> {
    let done = sqlx::query!(
        "DELETE FROM blob_grants WHERE ctid IN (
             SELECT ctid FROM blob_grants WHERE expires_at <= now()
             ORDER BY expires_at LIMIT $1 FOR UPDATE SKIP LOCKED
         )",
        limit,
    )
    .execute(executor)
    .await?;
    Ok(done.rows_affected())
}

/// Up to `limit` blobs that have expired and that nothing holds: no job or
/// saved diff references them and no key holds a live grant for them.
pub async fn blob_sweep_candidates(
    executor: impl PgExecutor<'_>,
    limit: i64,
) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar!(
        "SELECT sha256 FROM blobs
         WHERE expires_at <= now() AND NOT diffyman_blob_held(sha256)
         ORDER BY expires_at LIMIT $1",
        limit,
    )
    .fetch_all(executor)
    .await?)
}

/// Deletes the rows of those of `candidates` that are still expired and
/// held by nothing, and returns their IDs.
pub async fn sweep_blobs(
    executor: impl PgExecutor<'_>,
    candidates: &[String],
) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar!(
        "DELETE FROM blobs WHERE sha256 IN (
             SELECT sha256 FROM blobs
             WHERE sha256 = ANY($1) AND expires_at <= now() AND NOT diffyman_blob_held(sha256)
             FOR UPDATE SKIP LOCKED
         )
         RETURNING sha256",
        candidates,
    )
    .fetch_all(executor)
    .await?)
}
