//! `POST /v1/blobs`.

use axum::extract::{RawQuery, Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use diffyman_storage::BlobId;
use futures::{StreamExt, TryStreamExt};
use serde::Serialize;
use utoipa::ToSchema;

use super::query_param;
use crate::content::{StoredBlob, expiry, store_stream};
use crate::error::{ApiError, ErrorBody};
use crate::guard::Caller;
use crate::state::AppState;

/// A stored blob.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlobView {
    /// The blob ID: the lowercase hex SHA-256 of its bytes.
    #[schema(example = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")]
    pub blob_id: String,
    /// Size in bytes.
    pub size: u64,
    /// Media type given at this key's first upload of these bytes.
    pub media_type: Option<String>,
    /// When this key first uploaded these bytes.
    pub created_at: DateTime<Utc>,
    /// When this key's upload expires: after that the key can no longer
    /// start jobs on it, and it is removed unless a job, a saved diff or
    /// another key's upload still holds it. Uploading the same bytes again
    /// extends it.
    pub expires_at: DateTime<Utc>,
}

impl From<StoredBlob> for BlobView {
    fn from(blob: StoredBlob) -> BlobView {
        BlobView {
            blob_id: blob.id.to_hex(),
            size: blob.size,
            media_type: blob.media_type,
            created_at: blob.created_at,
            expires_at: blob.expires_at,
        }
    }
}

/// Raw bytes.
#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
pub struct BinaryBody(pub Vec<u8>);

/// A recorded media type: printable ASCII, at most 255 characters.
fn recorded_media_type(request: &Request) -> Option<String> {
    let value = request.headers().get(header::CONTENT_TYPE)?.to_str().ok()?;
    let value = value.trim();
    let printable = value.bytes().all(|b| (0x20..0x7f).contains(&b));
    (printable && (1..=255).contains(&value.len())).then(|| value.to_owned())
}

/// Uploads a blob. The body is streamed to the store while it is hashed;
/// the blob ID is its SHA-256. Blobs belong to the key that uploaded them:
/// only it can start jobs on them, and the answer never shows whether
/// another key stored the same bytes.
#[utoipa::path(
    post,
    path = "/v1/blobs",
    tag = "blobs",
    params(("sha256" = Option<String>, Query, description = "Expected SHA-256 (64 lowercase hex digits); the upload is refused unless the bytes match")),
    request_body(content = BinaryBody, content_type = "application/octet-stream", description = "The bytes, at most the blob limit (1 GiB by default). Content-Type is recorded as the media type."),
    responses(
        (status = 201, description = "Stored for this key", body = BlobView),
        (status = 200, description = "This key had already uploaded these bytes; the expiry is extended", body = BlobView),
        (status = 400, description = "Invalid `sha256`, hash mismatch or a broken body", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 413, description = "Larger than the blob limit", body = ErrorBody),
        (status = 503, description = "A dependency is down", body = ErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn upload_blob(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    RawQuery(query): RawQuery,
    request: Request,
) -> Result<Response, ApiError> {
    let expected = match query_param(query.as_deref(), "sha256")? {
        None => None,
        Some(text) => Some(text.parse::<BlobId>()?),
    };
    let limit = state.store.max_blob_bytes();
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > limit) {
        return Err(ApiError::too_large(
            "blob-too-large",
            format!("blob is larger than the limit of {limit} bytes"),
        ));
    }
    let media_type = recorded_media_type(&request);
    let stream = request
        .into_body()
        .into_data_stream()
        .map_err(std::io::Error::other)
        .boxed();
    let expires = expiry(state.config.retention.blob_ttl);
    let blob = store_stream(
        &state,
        stream,
        expected,
        media_type.as_deref(),
        expires,
        caller.key_id(),
    )
    .await?;
    let status = if blob.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    tracing::info!(blob_id = %blob.id, size = blob.size, "blob stored");
    Ok((status, Json(BlobView::from(blob))).into_response())
}
