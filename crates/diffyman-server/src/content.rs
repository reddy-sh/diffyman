//! Storing content: a blob object plus its row.
//!
//! The object is written first, then the row is inserted (or its expiry
//! extended) under the blob's shared lock, and for an upload the caller's
//! grant with it. A failure in between leaves an object without a row, which
//! the sweeper removes after the orphan grace period. When this call
//! inserted the row, the object is checked once more: a delete or a sweep
//! that removed the row and its object between the write and the insert
//! would otherwise leave a row without an object, so the upload is refused
//! as `storage-conflict` and can be retried. Deleters keep the blob's lock
//! until the object is gone (see `diffyman_db::blobs`), so the check cannot
//! miss one.

use bytes::Bytes;
use chrono::{DateTime, Utc};
use diffyman_db::blobs;
use diffyman_storage::{BlobId, InputStream, PutOutcome};

use crate::error::ApiError;
use crate::state::AppState;

/// A stored blob.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredBlob {
    /// Its ID.
    pub id: BlobId,
    /// Size in bytes.
    pub size: u64,
    /// Media type recorded at its first upload.
    pub media_type: Option<String>,
    /// When its row was first written.
    pub created_at: DateTime<Utc>,
    /// When it may be removed once nothing references it.
    pub expires_at: DateTime<Utc>,
    /// Whether this call created the row or, for an upload, the caller's
    /// grant: whether the bytes are new for this caller.
    pub created: bool,
}

/// Stores bytes held in memory, for a saved diff or a job result: the
/// reference from the saved diff holds them, not a grant.
pub async fn store_bytes(
    state: &AppState,
    data: Bytes,
    media_type: Option<&str>,
    expires_at: DateTime<Utc>,
) -> Result<StoredBlob, ApiError> {
    let put = state.store.put_bytes(data, None).await?;
    record(state, put, media_type, expires_at, Holder::Reference).await
}

/// Stores an upload of `owner` (the caller's key ID), checking the size
/// limit and, with `expected`, the hash, and grants it to `owner`. The
/// returned dates and media type are the caller's grant's, so nothing tells
/// a caller whether another key stored the same bytes.
pub async fn store_stream(
    state: &AppState,
    data: InputStream,
    expected: Option<BlobId>,
    media_type: Option<&str>,
    expires_at: DateTime<Utc>,
    owner: Option<uuid::Uuid>,
) -> Result<StoredBlob, ApiError> {
    let put = state.store.put_stream(data, expected).await?;
    record(state, put, media_type, expires_at, Holder::Upload(owner)).await
}

/// What keeps a stored blob.
#[derive(Clone, Copy, Debug)]
enum Holder {
    /// The saved diff about to reference it.
    Reference,
    /// The grant of the uploading key (`None` when authentication is off).
    Upload(Option<uuid::Uuid>),
}

async fn record(
    state: &AppState,
    put: PutOutcome,
    media_type: Option<&str>,
    expires_at: DateTime<Utc>,
    holder: Holder,
) -> Result<StoredBlob, ApiError> {
    let size = i64::try_from(put.size).map_err(|_| ApiError::internal())?;
    let hex = put.id.to_hex();
    let mut tx = state.db.begin().await?;
    blobs::lock_shared(&mut *tx, &hex).await?;
    let (row, inserted) = blobs::upsert(&mut *tx, &hex, size, media_type, expires_at).await?;
    let grant = match holder {
        Holder::Upload(owner) => {
            Some(blobs::grant(&mut *tx, &hex, owner, media_type, expires_at).await?)
        }
        Holder::Reference => None,
    };
    tx.commit().await.map_err(diffyman_db::DbError::from)?;
    if inserted && state.store.head(put.id).await?.is_none() {
        return Err(ApiError::unavailable(
            "storage-conflict",
            "the blob was removed while it was stored; send it again",
            1,
        ));
    }
    let stored = match grant {
        Some((grant, new)) => StoredBlob {
            id: put.id,
            size: put.size,
            media_type: grant.media_type,
            created_at: grant.created_at,
            expires_at: grant.expires_at,
            created: new,
        },
        None => StoredBlob {
            id: put.id,
            size: put.size,
            media_type: row.media_type,
            created_at: row.created_at,
            expires_at: row.expires_at,
            created: inserted,
        },
    };
    Ok(stored)
}

/// Deletes the objects of blob rows that were just deleted and committed,
/// while the caller still holds their delete locks. An object that cannot
/// be deleted now has no row, so the orphan sweep removes it later; that is
/// logged, never returned as an error. Returns how many were deleted.
pub async fn delete_objects(state: &AppState, ids: &[String]) -> usize {
    let mut deleted = 0;
    for id in ids {
        let Ok(blob) = id.parse::<BlobId>() else {
            continue;
        };
        match state.store.delete(blob).await {
            Ok(()) => deleted += 1,
            Err(error) => tracing::warn!(
                blob_id = %blob,
                code = error.code(),
                "deleting an object failed; the orphan sweep removes it later"
            ),
        }
    }
    deleted
}

/// A time `ttl` from now.
pub fn expiry(ttl: std::time::Duration) -> DateTime<Utc> {
    let ttl = chrono::TimeDelta::from_std(ttl).unwrap_or(chrono::TimeDelta::days(30));
    Utc::now() + ttl
}

/// Reads a whole blob into memory, refusing one over `limit` bytes.
pub async fn read_blob(state: &AppState, id: BlobId, limit: u64) -> Result<Bytes, ApiError> {
    use futures::TryStreamExt;
    let (meta, mut stream) = state.store.get(id).await?;
    if meta.size > limit {
        return Err(ApiError::too_large(
            "job-too-large",
            format!("the inputs are larger than the job limit of {limit} bytes"),
        ));
    }
    let mut out = Vec::with_capacity(usize::try_from(meta.size).unwrap_or(0));
    while let Some(chunk) = stream.try_next().await? {
        if (out.len() + chunk.len()) as u64 > limit {
            return Err(ApiError::too_large(
                "job-too-large",
                format!("the inputs are larger than the job limit of {limit} bytes"),
            ));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(out))
}
