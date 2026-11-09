//! Content-addressed blob storage for `diffyman-server`
//! ([ADR 0004](../../../docs/adr/0004-object-storage.md),
//! [ADR 0012](../../../docs/adr/0012-local-service-architecture.md)).
//!
//! A blob is named by the SHA-256 of its bytes ([`BlobId`]) and stored at
//! `blobs/sha256/<first two hex digits>/<64 hex digits>`. Writing the same
//! blob twice is harmless. [`BlobStore`] is the only interface the server
//! sees; [`open`] builds one from a store URL:
//!
//! - `file:///absolute/folder`: a local folder. Every object is written to a
//!   temporary file and renamed into place, so readers never see a partial
//!   blob.
//! - `memory://`: in memory, for tests.
//! - `s3://bucket/optional/prefix`: S3 or an S3-compatible server, only with
//!   the `s3` feature.
//!
//! A streamed put writes to `staging/<uuid>` while it hashes and counts the
//! bytes, and renames the object to its content address only when the size
//! is within the limit and the hash matches the expected one. A put from
//! bytes hashes first and writes the final key directly.

use std::fmt;
use std::ops::Range;
use std::str::FromStr;
use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use futures::stream::{self, BoxStream, StreamExt, TryStreamExt};
use object_store::path::Path;
use object_store::{
    GetOptions, ObjectStore, ObjectStoreExt, PutMode, PutOptions, PutPayload, WriteMultipart,
};
use sha2::{Digest, Sha256};

mod open;

pub use open::{StoreOptions, open};

/// Key prefix of content-addressed blobs.
pub const BLOBS_PREFIX: &str = "blobs/sha256";
/// Key prefix of streamed uploads that are not verified yet.
pub const STAGING_PREFIX: &str = "staging";

/// Uploads with this many parts in flight wait for one to finish.
const MAX_PARTS_IN_FLIGHT: usize = 4;

/// Result alias for storage operations.
pub type Result<T, E = StorageError> = std::result::Result<T, E>;

/// A stream of blob bytes.
pub type ByteStream = BoxStream<'static, Result<Bytes>>;

/// A stream of bytes to store, such as an HTTP request body.
pub type InputStream = BoxStream<'static, std::io::Result<Bytes>>;

/// Errors from a blob store. [`StorageError::code`] gives a stable code for
/// API errors and logs; `Display` never contains blob content, file names or
/// paths.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// The text is not 64 lowercase hex digits.
    #[error("invalid blob id: expected 64 lowercase hex digits")]
    InvalidId,
    /// No blob with this ID is stored.
    #[error("blob not found")]
    NotFound,
    /// The blob is larger than the store's limit.
    #[error("blob is larger than the limit of {limit} bytes")]
    TooLarge {
        /// The limit in bytes.
        limit: u64,
    },
    /// The content does not hash to the expected blob ID.
    #[error("content does not match the expected SHA-256")]
    HashMismatch,
    /// The byte range is empty-reversed or extends past the end of the blob.
    #[error("byte range is not satisfiable")]
    InvalidRange,
    /// The store URL could not be used.
    #[error("invalid store URL: {0}")]
    InvalidUrl(&'static str),
    /// The input stream failed.
    #[error("reading the input failed")]
    Input(#[source] std::io::Error),
    /// The storage backend failed. The source may name local paths, so it is
    /// never part of `Display`.
    #[error("storage backend error")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl StorageError {
    /// A stable kebab-case code.
    pub fn code(&self) -> &'static str {
        match self {
            StorageError::InvalidId => "invalid-blob-id",
            StorageError::NotFound => "blob-not-found",
            StorageError::TooLarge { .. } => "blob-too-large",
            StorageError::HashMismatch => "hash-mismatch",
            StorageError::InvalidRange => "invalid-range",
            StorageError::InvalidUrl(_) => "invalid-store-url",
            StorageError::Input(_) => "input-error",
            StorageError::Backend(_) => "storage-unavailable",
        }
    }
}

fn backend(error: object_store::Error) -> StorageError {
    match error {
        object_store::Error::NotFound { .. } => StorageError::NotFound,
        other => StorageError::Backend(Box::new(other)),
    }
}

/// The SHA-256 of a blob's bytes, shown as 64 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobId([u8; 32]);

impl BlobId {
    /// The ID of these bytes.
    pub fn of(bytes: &[u8]) -> BlobId {
        BlobId(Sha256::digest(bytes).into())
    }

    /// Wraps a raw SHA-256 digest.
    pub fn from_digest(digest: [u8; 32]) -> BlobId {
        BlobId(digest)
    }

    /// The raw digest.
    pub fn digest(&self) -> &[u8; 32] {
        &self.0
    }

    /// The 64 lowercase hex digits.
    pub fn to_hex(&self) -> String {
        self.to_string()
    }

    /// The object key: `blobs/sha256/ab/<hex>`.
    pub fn key(&self) -> String {
        let hex = self.to_hex();
        format!("{BLOBS_PREFIX}/{}/{hex}", &hex[..2])
    }

    fn path(&self) -> Path {
        Path::from(self.key())
    }

    /// Parses an object key written by [`BlobId::key`].
    pub fn from_key(key: &str) -> Option<BlobId> {
        let rest = key.strip_prefix(BLOBS_PREFIX)?.strip_prefix('/')?;
        let (fan, hex) = rest.split_once('/')?;
        let id = hex.parse::<BlobId>().ok()?;
        (fan.len() == 2 && hex.starts_with(fan)).then_some(id)
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for BlobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlobId({self})")
    }
}

impl FromStr for BlobId {
    type Err = StorageError;

    fn from_str(text: &str) -> Result<BlobId> {
        let bytes = text.as_bytes();
        if bytes.len() != 64 {
            return Err(StorageError::InvalidId);
        }
        let mut digest = [0u8; 32];
        for (out, pair) in digest.iter_mut().zip(bytes.as_chunks::<2>().0) {
            *out = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Ok(BlobId(digest))
    }
}

fn nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(StorageError::InvalidId),
    }
}

/// Size of a stored blob.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlobMeta {
    /// The blob's ID.
    pub id: BlobId,
    /// Its size in bytes.
    pub size: u64,
}

/// The result of a put.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PutOutcome {
    /// The blob's ID.
    pub id: BlobId,
    /// Its size in bytes.
    pub size: u64,
    /// `false` when the blob was already stored.
    pub created: bool,
}

/// Which part of the store to list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    /// Content-addressed blobs.
    Blobs,
    /// Streamed uploads that were never finished.
    Staging,
}

/// What an object key holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    /// A content-addressed blob.
    Blob(BlobId),
    /// An unfinished streamed upload, by key.
    Staging(String),
    /// A key that does not follow the layout. Sweepers leave it alone.
    Other(String),
}

/// An object found by [`BlobStore::list`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredObject {
    /// What the key holds.
    pub kind: ObjectKind,
    /// Size in bytes.
    pub size: u64,
    /// When the object was last written.
    pub last_modified: DateTime<Utc>,
}

/// Blob storage used by the server. Object-safe, so the server holds an
/// `Arc<dyn BlobStore>` and tests swap in `memory://`.
pub trait BlobStore: Send + Sync + fmt::Debug {
    /// The largest blob this store accepts, in bytes.
    fn max_blob_bytes(&self) -> u64;

    /// Stores `data`. With `expected`, fails with
    /// [`StorageError::HashMismatch`] unless the content hashes to it.
    fn put_bytes(&self, data: Bytes, expected: Option<BlobId>)
    -> BoxFuture<'_, Result<PutOutcome>>;

    /// Stores a stream without holding it in memory; see the crate docs.
    fn put_stream(
        &self,
        data: InputStream,
        expected: Option<BlobId>,
    ) -> BoxFuture<'_, Result<PutOutcome>>;

    /// The blob's size and a stream of its bytes.
    fn get(&self, id: BlobId) -> BoxFuture<'_, Result<(BlobMeta, ByteStream)>>;

    /// A stream of `range` of the blob. An empty range gives an empty stream;
    /// a range past the end gives [`StorageError::InvalidRange`].
    fn get_range(&self, id: BlobId, range: Range<u64>) -> BoxFuture<'_, Result<ByteStream>>;

    /// The blob's size, or `None` when it is not stored.
    fn head(&self, id: BlobId) -> BoxFuture<'_, Result<Option<BlobMeta>>>;

    /// Deletes the blob. Deleting a missing blob succeeds.
    fn delete(&self, id: BlobId) -> BoxFuture<'_, Result<()>>;

    /// Every object in `area`, for sweeps.
    fn list(&self, area: Area) -> BoxStream<'_, Result<StoredObject>>;

    /// Deletes an object found by [`BlobStore::list`]. `Other` keys are
    /// refused with [`StorageError::InvalidId`].
    fn delete_object<'a>(&'a self, kind: &'a ObjectKind) -> BoxFuture<'a, Result<()>>;
}

/// A [`BlobStore`] over any `object_store` backend.
#[derive(Debug, Clone)]
pub struct ObjectBlobStore {
    store: Arc<dyn ObjectStore>,
    options: StoreOptions,
}

impl ObjectBlobStore {
    /// Wraps an object store.
    pub(crate) fn new(store: Arc<dyn ObjectStore>, options: StoreOptions) -> ObjectBlobStore {
        ObjectBlobStore { store, options }
    }

    fn check_size(&self, size: u64) -> Result<()> {
        if size > self.options.max_blob_bytes {
            Err(StorageError::TooLarge {
                limit: self.options.max_blob_bytes,
            })
        } else {
            Ok(())
        }
    }

    async fn exists(&self, path: &Path) -> Result<bool> {
        match self.store.head(path).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(other) => Err(backend(other)),
        }
    }

    async fn put_bytes_inner(&self, data: Bytes, expected: Option<BlobId>) -> Result<PutOutcome> {
        let size = data.len() as u64;
        self.check_size(size)?;
        let id = BlobId::of(&data);
        if expected.is_some_and(|e| e != id) {
            return Err(StorageError::HashMismatch);
        }
        let path = id.path();
        if self.exists(&path).await? {
            return Ok(PutOutcome {
                id,
                size,
                created: false,
            });
        }
        let options = PutOptions {
            mode: PutMode::Overwrite,
            ..PutOptions::default()
        };
        self.store
            .put_opts(&path, PutPayload::from(data), options)
            .await
            .map_err(backend)?;
        Ok(PutOutcome {
            id,
            size,
            created: true,
        })
    }

    async fn put_stream_inner(
        &self,
        mut input: InputStream,
        expected: Option<BlobId>,
    ) -> Result<PutOutcome> {
        let chunk_size = self.options.part_bytes.max(1);
        let mut hasher = Sha256::new();
        let mut size: u64 = 0;
        // Small inputs are buffered and stored with one put; larger ones go
        // to a staging key as a multipart upload.
        let mut buffered: Vec<Bytes> = Vec::new();
        let mut buffered_len = 0usize;
        let mut upload: Option<(Path, WriteMultipart)> = None;

        loop {
            let next = match input.next().await {
                None => break,
                Some(Ok(chunk)) => chunk,
                Some(Err(error)) => {
                    abort(upload).await;
                    return Err(StorageError::Input(error));
                }
            };
            size += next.len() as u64;
            if let Err(error) = self.check_size(size) {
                abort(upload).await;
                return Err(error);
            }
            hasher.update(&next);
            if let Some((_, writer)) = upload.as_mut() {
                if let Err(error) = writer.wait_for_capacity(MAX_PARTS_IN_FLIGHT).await {
                    abort(upload).await;
                    return Err(backend(error));
                }
                writer.put(next);
                continue;
            }
            buffered_len += next.len();
            buffered.push(next);
            if buffered_len > chunk_size {
                let path = Path::from(format!("{STAGING_PREFIX}/{}", uuid::Uuid::new_v4()));
                let multipart = self.store.put_multipart(&path).await.map_err(backend)?;
                let mut writer = WriteMultipart::new_with_chunk_size(multipart, chunk_size);
                for part in buffered.drain(..) {
                    writer.put(part);
                }
                upload = Some((path, writer));
            }
        }

        let id = BlobId::from_digest(hasher.finalize().into());
        if expected.is_some_and(|e| e != id) {
            abort(upload).await;
            return Err(StorageError::HashMismatch);
        }
        let Some((staging, writer)) = upload else {
            let data: Bytes = if buffered.len() == 1 {
                buffered.pop().unwrap_or_default()
            } else {
                buffered.concat().into()
            };
            return self.put_bytes_inner(data, Some(id)).await;
        };
        writer.finish().await.map_err(backend)?;
        let target = id.path();
        let created = if self.exists(&target).await? {
            false
        } else {
            match self.store.rename(&staging, &target).await {
                Ok(()) => true,
                Err(error) => {
                    let _ = self.store.delete(&staging).await;
                    return Err(backend(error));
                }
            }
        };
        if !created {
            self.store.delete(&staging).await.map_err(backend)?;
        }
        Ok(PutOutcome { id, size, created })
    }

    async fn head_inner(&self, id: BlobId) -> Result<Option<BlobMeta>> {
        match self.store.head(&id.path()).await {
            Ok(meta) => Ok(Some(BlobMeta {
                id,
                size: meta.size,
            })),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(other) => Err(backend(other)),
        }
    }

    async fn get_inner(&self, id: BlobId) -> Result<(BlobMeta, ByteStream)> {
        let result = self.store.get(&id.path()).await.map_err(backend)?;
        let meta = BlobMeta {
            id,
            size: result.meta.size,
        };
        Ok((meta, result.into_stream().map_err(backend).boxed()))
    }

    async fn get_range_inner(&self, id: BlobId, range: Range<u64>) -> Result<ByteStream> {
        if range.start > range.end {
            return Err(StorageError::InvalidRange);
        }
        let meta = self.head_inner(id).await?.ok_or(StorageError::NotFound)?;
        if range.end > meta.size {
            return Err(StorageError::InvalidRange);
        }
        if range.is_empty() {
            return Ok(stream::empty().boxed());
        }
        let options = GetOptions::new().with_range(Some(range));
        let result = self
            .store
            .get_opts(&id.path(), options)
            .await
            .map_err(backend)?;
        Ok(result.into_stream().map_err(backend).boxed())
    }

    async fn delete_path(&self, path: &Path) -> Result<()> {
        match self.store.delete(path).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(other) => Err(backend(other)),
        }
    }
}

async fn abort(upload: Option<(Path, WriteMultipart)>) {
    if let Some((_, writer)) = upload {
        // Best effort: an upload that cannot be aborted stays under
        // `staging/`, where the sweeper removes it.
        let _ = writer.abort().await;
    }
}

impl BlobStore for ObjectBlobStore {
    fn max_blob_bytes(&self) -> u64 {
        self.options.max_blob_bytes
    }

    fn put_bytes(
        &self,
        data: Bytes,
        expected: Option<BlobId>,
    ) -> BoxFuture<'_, Result<PutOutcome>> {
        Box::pin(self.put_bytes_inner(data, expected))
    }

    fn put_stream(
        &self,
        data: InputStream,
        expected: Option<BlobId>,
    ) -> BoxFuture<'_, Result<PutOutcome>> {
        Box::pin(self.put_stream_inner(data, expected))
    }

    fn get(&self, id: BlobId) -> BoxFuture<'_, Result<(BlobMeta, ByteStream)>> {
        Box::pin(self.get_inner(id))
    }

    fn get_range(&self, id: BlobId, range: Range<u64>) -> BoxFuture<'_, Result<ByteStream>> {
        Box::pin(self.get_range_inner(id, range))
    }

    fn head(&self, id: BlobId) -> BoxFuture<'_, Result<Option<BlobMeta>>> {
        Box::pin(self.head_inner(id))
    }

    fn delete(&self, id: BlobId) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { self.delete_path(&id.path()).await })
    }

    fn list(&self, area: Area) -> BoxStream<'_, Result<StoredObject>> {
        let prefix = Path::from(match area {
            Area::Blobs => BLOBS_PREFIX,
            Area::Staging => STAGING_PREFIX,
        });
        self.store
            .list(Some(&prefix))
            .map(move |item| {
                let meta = item.map_err(backend)?;
                let key = meta.location.to_string();
                let kind = match area {
                    Area::Blobs => match BlobId::from_key(&key) {
                        Some(id) => ObjectKind::Blob(id),
                        None => ObjectKind::Other(key),
                    },
                    Area::Staging => ObjectKind::Staging(key),
                };
                Ok(StoredObject {
                    kind,
                    size: meta.size,
                    last_modified: meta.last_modified,
                })
            })
            .boxed()
    }

    fn delete_object<'a>(&'a self, kind: &'a ObjectKind) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            match kind {
                ObjectKind::Blob(id) => self.delete_path(&id.path()).await,
                ObjectKind::Staging(key) if is_staging_key(key) => {
                    self.delete_path(&Path::from(key.as_str())).await
                }
                ObjectKind::Staging(_) | ObjectKind::Other(_) => Err(StorageError::InvalidId),
            }
        })
    }
}

fn is_staging_key(key: &str) -> bool {
    key.strip_prefix(STAGING_PREFIX)
        .and_then(|rest| rest.strip_prefix('/'))
        .is_some_and(|name| uuid::Uuid::parse_str(name).is_ok())
}

#[cfg(test)]
mod tests;
