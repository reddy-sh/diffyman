# 0004. Object storage behind a `BlobStore` trait

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

The REST API service stores blobs: uploaded inputs, Diff Document JSON and rendered outputs. Some are large (GB-scale inputs, big child documents in folder and archive diffs). Locally they should live in a plain folder on disk. In the cloud they will live in Amazon S3. The code must not change between the two, and large blobs must stream rather than load into memory.

Two Rust libraries fit:

| | `object_store` 0.14.2 | OpenDAL 0.59.5 |
|---|---|---|
| License | MIT OR Apache-2.0 | Apache-2.0 |
| Backends | Local fs, memory, S3, GCS, Azure, HTTP | 50+ |
| Streaming / multipart | `put_multipart`, `WriteMultipart`, `into_stream` | `writer_with` with chunk and concurrency options |
| Presigned URLs | `Signer` (S3, GCS, Azure) | `presign_read`, `presign_write` |
| Conditional put | `PutMode` + ETag | Not checked |
| Caveat | Pre-1.0, so expect breaking changes | Larger surface than we need |

We also need an S3-compatible server for local integration tests, so the S3 code path (multipart uploads, presigned URLs) is exercised before touching a real cloud account.

## Decision

- **Use `object_store`** (exact version pinned) for local disk and S3. Its scope is small and covers what we need.
- **Wrap it in our own `BlobStore` trait** in `diffyman-storage`: streaming put and get, head, delete, and optional presign. Nothing outside that crate uses `object_store` types, so OpenDAL or another backend can be swapped in later.
- **Configuration is one URL:**
  - `file://` pointing at `~/diffyman-data/blobs` for normal local runs;
  - `memory://` for unit tests;
  - `s3://<bucket>/` in the cloud.
- **Content-addressed keys** (`blobs/sha256/ab/<hash>`). Writing the same blob twice is harmless, so we need no conditional writes.
- **Presigned URLs only on S3.** In local mode, downloads stream through the REST API.
- **S3 integration tests run against RustFS** (Apache-2.0, pinned to 1.0.4 or later), in the optional `s3` profile of `deploy/local/compose.yaml`, published on `127.0.0.1` only, with non-default credentials from `.env.local`.
- **Not MinIO.** Its repository was archived in April 2026, it now ships as source only, and it is AGPL-3.0.
- **Fallbacks if RustFS fails:** SeaweedFS or Versity Gateway (both Apache-2.0). The final check is always a test run against real S3 in a staging account.

## Consequences

- Moving to S3 is a configuration change (`file://` to `s3://`) plus credentials, not a code change.
- The engine, CLI and bindings never depend on `object_store`. Only the server-side crates do.
- We own the `BlobStore` trait and its tests, and must track `object_store` breaking changes until it reaches 1.0.
- RustFS is young. Conditional writes and versioning are only partly supported, and early releases had a critical security bug. So we pin a fixed version, never use its default credentials, and keep it test-only.
- Local blobs are ordinary files under `~/diffyman-data/blobs`, outside the repository. They need their own backup alongside the database dump.
- Ranged reads must work on both backends, because large-input handling streams from storage.
