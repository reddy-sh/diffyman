//! Blob rows (one per object in the blob store) and blob grants (one per
//! key that uploaded the blob).
//!
//! Ordering rule (ADR 0012): a row exists before anything references its
//! object, and a row is deleted, and committed, before its object is
//! deleted. An advisory lock per blob orders the two: a writer stores the
//! row under [`lock_shared`] and then checks that the object is still there;
//! a deleter holds [`lock_for_delete`] from before it deletes the row until
//! its object is gone. So a row never points at a missing object for longer
//! than an upload takes, and a failed object deletion leaves only an object
//! without a row, which the orphan sweep removes.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::{PgConnection, Result};

/// A stored blob's metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blob {
    /// Lowercase hex SHA-256: the blob ID and the object's name.
    pub sha256: String,
    /// Size in bytes.
    pub size: i64,
    /// Media type given at upload, if any.
    pub media_type: Option<String>,
    /// When the row was first written.
    pub created_at: DateTime<Utc>,
    /// When the blob may be removed, once nothing references it.
    pub expires_at: DateTime<Utc>,
}

/// One key's hold on a blob it uploaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// The blob.
    pub sha256: String,
    /// The key, or `None` when authentication is off.
    pub api_key_id: Option<Uuid>,
    /// Media type given at this key's first upload, if any.
    pub media_type: Option<String>,
    /// When this key first uploaded the blob.
    pub created_at: DateTime<Utc>,
    /// When this key's hold ends.
    pub expires_at: DateTime<Utc>,
}

/// Inserts a blob row, or extends the expiry of an existing one. Returns the
/// row and whether it was inserted.
pub async fn upsert(
    executor: impl PgExecutor<'_>,
    sha256: &str,
    size: i64,
    media_type: Option<&str>,
    expires_at: DateTime<Utc>,
) -> Result<(Blob, bool)> {
    let row = sqlx::query!(
        "INSERT INTO blobs (sha256, size, media_type, expires_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (sha256) DO UPDATE
             SET expires_at = GREATEST(blobs.expires_at, EXCLUDED.expires_at)
         RETURNING sha256, size, media_type, created_at, expires_at, (xmax = 0) AS \"inserted!\"",
        sha256,
        size,
        media_type,
        expires_at,
    )
    .fetch_one(executor)
    .await?;
    let blob = Blob {
        sha256: row.sha256,
        size: row.size,
        media_type: row.media_type,
        created_at: row.created_at,
        expires_at: row.expires_at,
    };
    Ok((blob, row.inserted))
}

/// Gives `api_key_id` a hold on an existing blob row until `expires_at`, or
/// extends its hold. Returns the grant and whether it is new for this key.
pub async fn grant(
    executor: impl PgExecutor<'_>,
    sha256: &str,
    api_key_id: Option<Uuid>,
    media_type: Option<&str>,
    expires_at: DateTime<Utc>,
) -> Result<(Grant, bool)> {
    let row = sqlx::query!(
        "INSERT INTO blob_grants (sha256, api_key_id, media_type, expires_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT ON CONSTRAINT blob_grants_once DO UPDATE
             SET expires_at = GREATEST(blob_grants.expires_at, EXCLUDED.expires_at),
                 -- An expired hold that is taken up again counts as new.
                 created_at = CASE WHEN blob_grants.expires_at <= now() THEN now()
                                   ELSE blob_grants.created_at END
         RETURNING sha256, api_key_id, media_type, created_at, expires_at,
                   (xmax = 0 OR created_at = now()) AS \"inserted!\"",
        sha256,
        api_key_id,
        media_type,
        expires_at,
    )
    .fetch_one(executor)
    .await?;
    let grant = Grant {
        sha256: row.sha256,
        api_key_id: row.api_key_id,
        media_type: row.media_type,
        created_at: row.created_at,
        expires_at: row.expires_at,
    };
    Ok((grant, row.inserted))
}

/// The blob row, or `None`.
pub async fn get(executor: impl PgExecutor<'_>, sha256: &str) -> Result<Option<Blob>> {
    Ok(sqlx::query_as!(
        Blob,
        "SELECT sha256, size, media_type, created_at, expires_at FROM blobs WHERE sha256 = $1",
        sha256,
    )
    .fetch_optional(executor)
    .await?)
}

/// The blob row when `api_key_id` holds a live grant for it, else `None`:
/// a key can use only the blobs it uploaded itself.
pub async fn granted(
    executor: impl PgExecutor<'_>,
    sha256: &str,
    api_key_id: Option<Uuid>,
) -> Result<Option<Blob>> {
    Ok(sqlx::query_as!(
        Blob,
        "SELECT b.sha256, b.size, b.media_type, b.created_at, b.expires_at
         FROM blobs b JOIN blob_grants g ON g.sha256 = b.sha256
         WHERE b.sha256 = $1 AND g.api_key_id IS NOT DISTINCT FROM $2 AND g.expires_at > now()",
        sha256,
        api_key_id,
    )
    .fetch_optional(executor)
    .await?)
}

/// Which of `candidates` have a row; the rest are objects without one.
pub async fn existing(executor: impl PgExecutor<'_>, candidates: &[String]) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar!(
        "SELECT sha256 FROM blobs WHERE sha256 = ANY($1)",
        candidates
    )
    .fetch_all(executor)
    .await?)
}

/// Whether a job or a saved diff references the blob.
pub async fn in_use(executor: impl PgExecutor<'_>, sha256: &str) -> Result<bool> {
    Ok(
        sqlx::query_scalar!("SELECT diffyman_blob_in_use($1) AS \"in_use!\"", sha256)
            .fetch_one(executor)
            .await?,
    )
}

/// Whether the blob is in use or a key holds a live grant for it: such a
/// blob is never deleted.
pub async fn held(executor: impl PgExecutor<'_>, sha256: &str) -> Result<bool> {
    Ok(
        sqlx::query_scalar!("SELECT diffyman_blob_held($1) AS \"held!\"", sha256)
            .fetch_one(executor)
            .await?,
    )
}

/// Takes the blob's lock in shared mode until the end of the transaction
/// `executor` runs in. Writers of the row take it; see the module docs.
pub async fn lock_shared(executor: impl PgExecutor<'_>, sha256: &str) -> Result<()> {
    sqlx::query_scalar!(
        "SELECT 1 AS \"one!\" FROM pg_advisory_xact_lock_shared(diffyman_blob_lock_key($1))",
        sha256,
    )
    .fetch_one(executor)
    .await?;
    Ok(())
}

/// Takes the locks of `blobs` in exclusive mode, in a fixed order, for the
/// rest of the session: hold them while deleting the rows and then the
/// objects. They are released only when the connection closes, so call this
/// on a connection that is closed afterwards, never one returned to a pool
/// (`PoolConnection::close_on_drop`).
pub async fn lock_for_delete(conn: &mut PgConnection, blobs: &[String]) -> Result<()> {
    let mut sorted: Vec<&String> = blobs.iter().collect();
    sorted.sort();
    sorted.dedup();
    for sha256 in sorted {
        sqlx::query_scalar!(
            "SELECT 1 AS \"one!\" FROM pg_advisory_lock(diffyman_blob_lock_key($1))",
            sha256,
        )
        .fetch_one(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Like [`lock_for_delete`], but skips blobs whose lock someone else holds
/// and returns the ones it locked.
pub async fn try_lock_for_delete(conn: &mut PgConnection, blobs: &[String]) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar!(
        "SELECT s AS \"sha256!\" FROM unnest($1::text[]) AS s
         WHERE pg_try_advisory_lock(diffyman_blob_lock_key(s))",
        blobs,
    )
    .fetch_all(&mut *conn)
    .await?)
}
