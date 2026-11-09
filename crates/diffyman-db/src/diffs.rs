//! Saved diffs: a stored Diff Document and its summary.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::{PgConnection, Result};

/// A saved diff.
#[derive(Clone, Debug, PartialEq)]
pub struct Diff {
    /// `UUIDv7`.
    pub id: Uuid,
    /// The key that created it.
    pub api_key_id: Option<Uuid>,
    /// Blob ID of the canonical Diff Document JSON.
    pub document_blob: String,
    /// Blob ID of the left input, when stored.
    pub left_blob: Option<String>,
    /// Blob ID of the right input, when stored.
    pub right_blob: Option<String>,
    /// The document's `status`.
    pub status: String,
    /// The document's `kind`.
    pub kind: String,
    /// The document's `stats` (`None` when its status is `error`).
    pub stats: Option<Value>,
    /// When it was saved.
    pub created_at: DateTime<Utc>,
    /// When it expires.
    pub expires_at: DateTime<Utc>,
}

/// A diff to save.
#[derive(Clone, Debug)]
pub struct NewDiff<'a> {
    /// The key saving it.
    pub api_key_id: Option<Uuid>,
    /// Blob ID of the document; the blob row must exist.
    pub document_blob: &'a str,
    /// Blob ID of the left input, if stored.
    pub left_blob: Option<&'a str>,
    /// Blob ID of the right input, if stored.
    pub right_blob: Option<&'a str>,
    /// The document's `status`.
    pub status: &'a str,
    /// The document's `kind`.
    pub kind: &'a str,
    /// The document's `stats`, a JSON object, or `None`.
    pub stats: Option<&'a Value>,
    /// When it expires.
    pub expires_at: DateTime<Utc>,
}

/// Saves a diff.
pub async fn create(executor: impl PgExecutor<'_>, new: &NewDiff<'_>) -> Result<Diff> {
    Ok(sqlx::query_as!(
        Diff,
        "INSERT INTO diffs (api_key_id, document_blob, left_blob, right_blob, status, kind, stats,
                            expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING *",
        new.api_key_id,
        new.document_blob,
        new.left_blob,
        new.right_blob,
        new.status,
        new.kind,
        new.stats,
        new.expires_at,
    )
    .fetch_one(executor)
    .await?)
}

/// The saved diff, or `None` when it does not exist or has expired.
pub async fn get(executor: impl PgExecutor<'_>, id: Uuid) -> Result<Option<Diff>> {
    Ok(sqlx::query_as!(
        Diff,
        "SELECT * FROM diffs WHERE id = $1 AND expires_at > now()",
        id
    )
    .fetch_optional(executor)
    .await?)
}

/// The blobs a saved diff and the jobs that produced it reference, sorted,
/// or `None` when the diff does not exist: the blobs [`delete`] may remove.
pub async fn blobs_of(conn: &mut PgConnection, id: Uuid) -> Result<Option<Vec<String>>> {
    let Some(diff) = sqlx::query!(
        "SELECT document_blob, left_blob, right_blob FROM diffs WHERE id = $1",
        id,
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    let mut blobs = vec![diff.document_blob];
    blobs.extend(diff.left_blob);
    blobs.extend(diff.right_blob);
    let jobs = sqlx::query!(
        "SELECT left_blob, right_blob, result_blob FROM jobs WHERE diff_id = $1",
        id,
    )
    .fetch_all(&mut *conn)
    .await?;
    for job in jobs {
        blobs.push(job.left_blob);
        blobs.push(job.right_blob);
        blobs.extend(job.result_blob);
    }
    blobs.sort();
    blobs.dedup();
    Ok(Some(blobs))
}

/// Deletes a saved diff, the jobs that produced it, and the rows of those
/// of `blobs` (from [`blobs_of`]) that nothing holds any more: no job or
/// saved diff references them and no key holds a live grant for them, so
/// another key's upload is never removed. Returns the deleted blob IDs, or
/// `None` when the diff does not exist.
///
/// Hold [`crate::blobs::lock_for_delete`] on `blobs` first, run this in a
/// transaction, commit, and only then delete the returned objects from the
/// blob store (ADR 0012): an object that cannot be deleted is left without a
/// row for the orphan sweep, and no row is ever left without its object.
pub async fn delete(
    conn: &mut PgConnection,
    id: Uuid,
    blobs: &[String],
) -> Result<Option<Vec<String>>> {
    let found = sqlx::query_scalar!("SELECT id FROM diffs WHERE id = $1 FOR UPDATE", id)
        .fetch_optional(&mut *conn)
        .await?;
    if found.is_none() {
        return Ok(None);
    }
    sqlx::query!("DELETE FROM jobs WHERE diff_id = $1", id)
        .execute(&mut *conn)
        .await?;
    sqlx::query!("DELETE FROM diffs WHERE id = $1", id)
        .execute(&mut *conn)
        .await?;
    let deleted = sqlx::query_scalar!(
        "DELETE FROM blobs
         WHERE sha256 IN (
             SELECT sha256 FROM blobs
             WHERE sha256 = ANY($1) AND NOT diffyman_blob_held(sha256)
             ORDER BY sha256
             FOR UPDATE
         )
         RETURNING sha256",
        blobs,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(Some(deleted))
}
