use std::sync::Arc;

use bytes::Bytes;
use futures::TryStreamExt;
use futures::stream::{self, StreamExt};
use url::Url;

use super::*;

const SMALL: StoreOptions = StoreOptions {
    max_blob_bytes: 64 * 1024,
    part_bytes: 1024,
};

struct Fixture {
    store: Arc<dyn BlobStore>,
    // Keeps the folder of a file:// store alive.
    folder: Option<tempfile::TempDir>,
}

fn memory() -> Fixture {
    Fixture {
        store: open("memory://", SMALL).unwrap(),
        folder: None,
    }
}

fn file() -> Fixture {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().join("blobs");
    let url = Url::from_directory_path(&root).unwrap();
    Fixture {
        store: open(url.as_str(), SMALL).unwrap(),
        folder: Some(folder),
    }
}

fn both() -> [Fixture; 2] {
    [memory(), file()]
}

fn chunks(data: &[u8], size: usize) -> InputStream {
    let parts: Vec<std::io::Result<Bytes>> = data
        .chunks(size)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();
    stream::iter(parts).boxed()
}

async fn read_all(stream: ByteStream) -> Vec<u8> {
    let parts: Vec<Bytes> = stream.try_collect().await.unwrap();
    parts.concat()
}

async fn list(store: &dyn BlobStore, area: Area) -> Vec<StoredObject> {
    store.list(area).try_collect().await.unwrap()
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from((i * 31 + i / 7) % 251).unwrap())
        .collect()
}

#[test]
fn blob_id_text_and_key() {
    let id = BlobId::of(b"abc");
    let hex = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert_eq!(id.to_hex(), hex);
    assert_eq!(id.key(), format!("blobs/sha256/ba/{hex}"));
    assert_eq!(hex.parse::<BlobId>().unwrap(), id);
    assert_eq!(BlobId::from_key(&id.key()), Some(id));
    assert_eq!(BlobId::from_key(&format!("blobs/sha256/bb/{hex}")), None);
    assert_eq!(BlobId::from_key(&format!("staging/{hex}")), None);
    for bad in [
        &hex.to_uppercase(),
        &hex[..63],
        &format!("{hex}0"),
        &hex.replace('a', "g"),
        "",
    ] {
        assert_eq!(
            bad.parse::<BlobId>().unwrap_err().code(),
            "invalid-blob-id",
            "{bad}"
        );
    }
}

#[tokio::test]
async fn put_bytes_round_trip_and_dedupe() {
    for fx in both() {
        let store = fx.store.as_ref();
        let data = Bytes::from_static(b"hello blob");
        let first = store.put_bytes(data.clone(), None).await.unwrap();
        assert_eq!(
            first,
            PutOutcome {
                id: BlobId::of(&data),
                size: 10,
                created: true
            }
        );
        let again = store.put_bytes(data.clone(), Some(first.id)).await.unwrap();
        assert!(!again.created);
        let (meta, body) = store.get(first.id).await.unwrap();
        assert_eq!(
            meta,
            BlobMeta {
                id: first.id,
                size: 10
            }
        );
        assert_eq!(read_all(body).await, data.to_vec());
        assert_eq!(store.head(first.id).await.unwrap(), Some(meta));
    }
}

#[tokio::test]
async fn file_store_uses_content_addressed_paths() {
    let fx = file();
    let out = fx
        .store
        .put_bytes(Bytes::from_static(b"on disk"), None)
        .await
        .unwrap();
    let hex = out.id.to_hex();
    let path = fx
        .folder
        .as_ref()
        .unwrap()
        .path()
        .join("blobs/blobs/sha256")
        .join(&hex[..2])
        .join(&hex);
    assert_eq!(std::fs::read(path).unwrap(), b"on disk");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let root = fx.folder.as_ref().unwrap().path().join("blobs");
        let mode = std::fs::metadata(root).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}

#[tokio::test]
async fn hash_mismatch_stores_nothing() {
    for fx in both() {
        let store = fx.store.as_ref();
        let wrong = BlobId::of(b"something else");
        let err = store
            .put_bytes(Bytes::from_static(b"data"), Some(wrong))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "hash-mismatch");
        let big = pattern(10_000);
        let err = store
            .put_stream(chunks(&big, 700), Some(wrong))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "hash-mismatch");
        assert!(list(store, Area::Blobs).await.is_empty());
        assert!(list(store, Area::Staging).await.is_empty());
    }
}

#[tokio::test]
async fn streamed_puts_small_and_multipart() {
    for fx in both() {
        let store = fx.store.as_ref();
        for len in [0usize, 1, 1024, 1025, 5000, 64 * 1024] {
            let data = pattern(len);
            let id = BlobId::of(&data);
            let out = store
                .put_stream(chunks(&data, 333), Some(id))
                .await
                .unwrap();
            assert_eq!(
                out,
                PutOutcome {
                    id,
                    size: len as u64,
                    created: true
                },
                "len {len}"
            );
            let again = store.put_stream(chunks(&data, 4096), None).await.unwrap();
            assert!(!again.created);
            let (meta, body) = store.get(id).await.unwrap();
            assert_eq!(meta.size, len as u64);
            assert_eq!(read_all(body).await, data, "len {len}");
        }
        assert!(list(store, Area::Staging).await.is_empty());
        assert_eq!(list(store, Area::Blobs).await.len(), 6);
    }
}

#[tokio::test]
async fn size_limit_applies_to_bytes_and_streams() {
    for fx in both() {
        let store = fx.store.as_ref();
        assert_eq!(store.max_blob_bytes(), 64 * 1024);
        let data = pattern(64 * 1024 + 1);
        let err = store
            .put_bytes(Bytes::from(data.clone()), None)
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::TooLarge { limit: 65536 }));
        let err = store
            .put_stream(chunks(&data, 1000), None)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "blob-too-large");
        assert!(list(store, Area::Blobs).await.is_empty());
        assert!(list(store, Area::Staging).await.is_empty());
    }
}

#[tokio::test]
async fn failing_input_leaves_no_staging_object() {
    for fx in both() {
        let store = fx.store.as_ref();
        let parts: Vec<std::io::Result<Bytes>> = vec![
            Ok(Bytes::from(pattern(3000))),
            Ok(Bytes::from(pattern(3000))),
            Err(std::io::Error::other("client went away")),
        ];
        let err = store
            .put_stream(stream::iter(parts).boxed(), None)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "input-error");
        assert_eq!(err.to_string(), "reading the input failed");
        assert!(list(store, Area::Staging).await.is_empty());
        assert!(list(store, Area::Blobs).await.is_empty());
    }
}

#[tokio::test]
async fn ranges() {
    for fx in both() {
        let store = fx.store.as_ref();
        let data = pattern(5000);
        let id = store
            .put_bytes(Bytes::from(data.clone()), None)
            .await
            .unwrap()
            .id;
        let part = read_all(store.get_range(id, 1000..1100).await.unwrap()).await;
        assert_eq!(part, data[1000..1100]);
        assert!(
            read_all(store.get_range(id, 5000..5000).await.unwrap())
                .await
                .is_empty()
        );
        assert_eq!(
            read_all(store.get_range(id, 0..5000).await.unwrap()).await,
            data
        );
        for bad in [4000..5001, 6000..6001] {
            let err = store.get_range(id, bad).await.err().unwrap();
            assert_eq!(err.code(), "invalid-range");
        }
        #[allow(clippy::reversed_empty_ranges)]
        let reversed = 10..5;
        assert_eq!(
            store.get_range(id, reversed).await.err().unwrap().code(),
            "invalid-range"
        );
        let missing = BlobId::of(b"missing");
        assert_eq!(
            store.get_range(missing, 0..1).await.err().unwrap().code(),
            "blob-not-found"
        );
    }
}

#[tokio::test]
async fn missing_blobs_and_idempotent_delete() {
    for fx in both() {
        let store = fx.store.as_ref();
        let missing = BlobId::of(b"missing");
        assert_eq!(store.head(missing).await.unwrap(), None);
        assert_eq!(
            store.get(missing).await.err().unwrap().code(),
            "blob-not-found"
        );
        store.delete(missing).await.unwrap();
        let id = store
            .put_bytes(Bytes::from_static(b"x"), None)
            .await
            .unwrap()
            .id;
        store.delete(id).await.unwrap();
        store.delete(id).await.unwrap();
        assert_eq!(store.head(id).await.unwrap(), None);
    }
}

#[tokio::test]
async fn listing_and_deleting_objects() {
    for fx in both() {
        let store = fx.store.as_ref();
        let a = store
            .put_bytes(Bytes::from_static(b"a"), None)
            .await
            .unwrap()
            .id;
        let b = store
            .put_stream(chunks(&pattern(3000), 500), None)
            .await
            .unwrap()
            .id;
        let mut found: Vec<BlobId> = list(store, Area::Blobs)
            .await
            .into_iter()
            .map(|o| match o.kind {
                ObjectKind::Blob(id) => id,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        found.sort();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(found, expected);
        store.delete_object(&ObjectKind::Blob(a)).await.unwrap();
        assert_eq!(list(store, Area::Blobs).await.len(), 1);
        let refused = store
            .delete_object(&ObjectKind::Other("blobs/x".into()))
            .await;
        assert_eq!(refused.unwrap_err().code(), "invalid-blob-id");
        let refused = store
            .delete_object(&ObjectKind::Staging("staging/../x".into()))
            .await;
        assert_eq!(refused.unwrap_err().code(), "invalid-blob-id");
    }
}

#[test]
fn store_urls() {
    assert!(open("memory://", StoreOptions::default()).is_ok());
    for (url, code) in [
        ("memory:///x", "invalid-store-url"),
        ("ftp://host/x", "invalid-store-url"),
        ("not a url", "invalid-store-url"),
        ("file:///tmp/x?y=1", "invalid-store-url"),
    ] {
        assert_eq!(
            open(url, StoreOptions::default()).unwrap_err().code(),
            code,
            "{url}"
        );
    }
    #[cfg(unix)]
    assert_eq!(
        open("file://host/share", StoreOptions::default())
            .unwrap_err()
            .code(),
        "invalid-store-url"
    );
    #[cfg(not(feature = "s3"))]
    assert_eq!(
        open("s3://bucket/", StoreOptions::default())
            .unwrap_err()
            .code(),
        "invalid-store-url"
    );
}

#[test]
fn errors_never_show_backend_details() {
    let error = StorageError::Backend(Box::new(std::io::Error::other("/secret/path")));
    assert_eq!(error.to_string(), "storage backend error");
    assert_eq!(error.code(), "storage-unavailable");
}
