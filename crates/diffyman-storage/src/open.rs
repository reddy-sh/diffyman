//! Building a [`BlobStore`] from a store URL.

use std::sync::Arc;

use object_store::ObjectStore;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use url::Url;

use crate::{BlobStore, ObjectBlobStore, Result, StorageError};

/// Limits and tuning shared by every backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreOptions {
    /// The largest blob accepted, in bytes.
    pub max_blob_bytes: u64,
    /// Part size of streamed uploads, in bytes. S3 needs at least 5 MiB.
    pub part_bytes: usize,
}

impl Default for StoreOptions {
    fn default() -> StoreOptions {
        StoreOptions {
            max_blob_bytes: 1 << 30,
            part_bytes: 8 << 20,
        }
    }
}

/// Opens the store at `url`: `file:///absolute/folder` (created with mode
/// 0700 when missing), `memory://`, or `s3://bucket/prefix` with the `s3`
/// feature.
pub fn open(url: &str, options: StoreOptions) -> Result<Arc<dyn BlobStore>> {
    let parsed = Url::parse(url).map_err(|_| StorageError::InvalidUrl("not a URL"))?;
    let store: Arc<dyn ObjectStore> = match parsed.scheme() {
        "memory" => {
            if !matches!(parsed.path(), "" | "/")
                || parsed.host_str().is_some_and(|h| !h.is_empty())
            {
                return Err(StorageError::InvalidUrl("memory:// takes no path"));
            }
            Arc::new(InMemory::new())
        }
        "file" => {
            if parsed.query().is_some() || parsed.fragment().is_some() {
                return Err(StorageError::InvalidUrl(
                    "file:// takes no query or fragment",
                ));
            }
            let folder = parsed
                .to_file_path()
                .map_err(|()| StorageError::InvalidUrl("file:// needs an absolute local path"))?;
            create_private_dir(&folder)?;
            let local = LocalFileSystem::new_with_prefix(&folder)
                .map_err(|e| StorageError::Backend(Box::new(e)))?
                .with_fsync(true)
                .with_automatic_cleanup(true);
            Arc::new(local)
        }
        "s3" => s3(&parsed)?,
        _ => return Err(StorageError::InvalidUrl("unsupported scheme")),
    };
    Ok(Arc::new(ObjectBlobStore::new(store, options)))
}

fn create_private_dir(folder: &std::path::Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(folder)
        .map_err(|e| StorageError::Backend(Box::new(e)))
}

#[cfg(feature = "s3")]
fn s3(url: &Url) -> Result<Arc<dyn ObjectStore>> {
    use object_store::aws::AmazonS3Builder;
    use object_store::prefix::PrefixStore;

    let bucket = url.host_str().filter(|b| !b.is_empty());
    let Some(bucket) = bucket else {
        return Err(StorageError::InvalidUrl("s3:// needs a bucket"));
    };
    let store = AmazonS3Builder::from_env()
        .with_bucket_name(bucket)
        .build()
        .map_err(|e| StorageError::Backend(Box::new(e)))?;
    let prefix = url.path().trim_matches('/');
    if prefix.is_empty() {
        Ok(Arc::new(store))
    } else {
        Ok(Arc::new(PrefixStore::new(store, prefix)))
    }
}

#[cfg(not(feature = "s3"))]
fn s3(_url: &Url) -> Result<Arc<dyn ObjectStore>> {
    Err(StorageError::InvalidUrl(
        "s3:// needs a build with the `s3` feature",
    ))
}
