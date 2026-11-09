//! `POST /v1/diff`, `GET /v1/diffs/{id}`, `DELETE /v1/diffs/{id}`.

use std::sync::Arc;

use axum::Extension;
use axum::body::Body;
use axum::extract::{FromRequest, Multipart, RawQuery, Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use bytes::Bytes;
use diffyman_db::diffs::{self, NewDiff};
use diffyman_db::records::{self, Actor, AuditEvent};
use diffyman_db::{Acquire, DbError, blobs};
use diffyman_storage::BlobId;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{IdPath, json_text, media_type, query_flag, read_limited};
use crate::content::{delete_objects, expiry, store_bytes};
use crate::engine::{self, DiffInputs, SYNC_MEMORY_PER_BODY_BYTE, Summary};
use crate::error::{ApiError, ErrorBody};
use crate::guard::Caller;
use crate::state::AppState;

/// Largest `options` part of a multipart request.
const MAX_OPTIONS_BYTES: u64 = 1 << 20;
/// Longest input name.
pub const MAX_NAME_CHARS: usize = 1024;

/// A Diff Document (schema 0.4.0) as canonical JSON (RFC 8785): the bytes
/// the engine wrote, identical to `diffyman diff --json` for the same inputs,
/// names and options except `provenance` (whose `binding` is `server`). See
/// `docs/spec/diff-document.md`.
#[derive(ToSchema)]
#[schema(as = DiffDocument, value_type = Object)]
pub struct DiffDocumentSchema(pub Value);

/// Two inputs and the options of a synchronous diff.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DiffRequest {
    /// The left (old) input.
    pub left: InputBody,
    /// The right (new) input.
    pub right: InputBody,
    /// An options object (`docs/spec/diff-document.md` §9); `{}` when absent.
    // Kept as its JSON text, for the engine's strict parser.
    #[schema(value_type = Option<Object>)]
    pub options: Option<Box<RawValue>>,
}

/// One input: exactly one of `text` or `base64`, and an optional name.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InputBody {
    /// File name; only its final component is kept. Used for type detection
    /// by extension and recorded in the document.
    pub name: Option<String>,
    /// The content as UTF-8 text.
    pub text: Option<String>,
    /// The content as standard base64 (with padding).
    pub base64: Option<String>,
}

/// A synchronous diff as `multipart/form-data`.
#[derive(ToSchema)]
pub struct DiffForm {
    /// The left input; its file name is its name.
    #[schema(value_type = String, format = Binary)]
    pub left: Vec<u8>,
    /// The right input.
    #[schema(value_type = String, format = Binary)]
    pub right: Vec<u8>,
    /// An options object as JSON text.
    pub options: Option<String>,
}

/// Checks an input name and keeps only its final path component, split on
/// `/` and `\` as the engine does (`diffyman::Input::named`); a name whose
/// final component is empty, such as `dir/`, counts as no name. `what` names
/// the field in the error, never the name itself.
pub fn input_name(what: &str, name: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(name) = name else {
        return Ok(None);
    };
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::invalid(format!(
            "{what} must have 1 to {MAX_NAME_CHARS} characters"
        )));
    }
    if name.contains('\0') {
        return Err(ApiError::nul_character(what));
    }
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    Ok((!last.is_empty()).then(|| last.to_owned()))
}

fn input_too_large(limit: u64) -> ApiError {
    ApiError::too_large(
        "input-too-large",
        format!(
            "an input of a synchronous diff is larger than {limit} bytes; upload blobs and create a job instead"
        ),
    )
}

fn decode(side: &str, input: InputBody, limit: u64) -> Result<(Bytes, Option<String>), ApiError> {
    let bytes = match (input.text, input.base64) {
        (Some(text), None) => Bytes::from(text),
        (None, Some(encoded)) => {
            // Base64 is 4/3 of the size; refuse before decoding.
            if (encoded.len() as u64) / 4 * 3 > limit + 2 {
                return Err(input_too_large(limit));
            }
            Bytes::from(
                base64::engine::general_purpose::STANDARD
                    .decode(encoded.as_bytes())
                    .map_err(|_| {
                        ApiError::invalid(format!("`{side}.base64` is not valid base64"))
                    })?,
            )
        }
        _ => {
            return Err(ApiError::invalid(format!(
                "`{side}` needs exactly one of `text` or `base64`"
            )));
        }
    };
    if bytes.len() as u64 > limit {
        return Err(input_too_large(limit));
    }
    Ok((bytes, input_name(&format!("`{side}.name`"), input.name)?))
}

async fn json_inputs(state: &AppState, request: Request) -> Result<DiffInputs, ApiError> {
    let limits = state.config.limits;
    let (parts, body) = request.into_parts();
    let body = read_limited(&parts.headers, body, limits.max_body_bytes).await?;
    let request: DiffRequest = super::parse_json(&parts.headers, &body)?;
    drop(body);
    let (left, left_name) = decode("left", request.left, limits.max_sync_input_bytes)?;
    let (right, right_name) = decode("right", request.right, limits.max_sync_input_bytes)?;
    Ok(DiffInputs {
        left,
        right,
        left_name,
        right_name,
        options: engine::options_text(request.options.as_deref().map(RawValue::get))?,
    })
}

fn multipart_error(error: &axum::extract::multipart::MultipartError) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::too_large("payload-too-large", "the request body is too large")
    } else {
        ApiError::invalid("the multipart body is not valid")
    }
}

async fn multipart_inputs(state: &AppState, request: Request) -> Result<DiffInputs, ApiError> {
    let limit = state.config.limits.max_sync_input_bytes;
    let mut form = Multipart::from_request(request, state)
        .await
        .map_err(|_| ApiError::invalid("the multipart body is not valid"))?;
    let mut sides: [Option<(Bytes, Option<String>)>; 2] = [None, None];
    let mut options: Option<String> = None;
    while let Some(mut field) = form.next_field().await.map_err(|e| multipart_error(&e))? {
        let which = match field.name() {
            Some("left") => Some(0),
            Some("right") => Some(1),
            Some("options") => None,
            _ => {
                return Err(ApiError::invalid(
                    "multipart fields are `left`, `right` and `options`",
                ));
            }
        };
        let name = field
            .file_name()
            .filter(|n| !n.is_empty())
            .map(str::to_owned);
        let cap = if which.is_some() {
            limit
        } else {
            MAX_OPTIONS_BYTES
        };
        let mut data = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(|e| multipart_error(&e))? {
            if (data.len() + chunk.len()) as u64 > cap {
                return Err(match which {
                    Some(_) => input_too_large(limit),
                    None => {
                        ApiError::too_large("payload-too-large", "the options part is too large")
                    }
                });
            }
            data.extend_from_slice(&chunk);
        }
        match which {
            Some(i) if sides[i].is_some() => {
                return Err(ApiError::invalid("each input is given once"));
            }
            Some(i) => {
                let what = format!("the file name of `{}`", ["left", "right"][i]);
                sides[i] = Some((Bytes::from(data), input_name(&what, name)?));
            }
            None if options.is_some() => {
                return Err(ApiError::invalid("`options` is given once"));
            }
            None => {
                options = Some(
                    String::from_utf8(data)
                        .map_err(|_| ApiError::invalid("`options` must be UTF-8 JSON"))?,
                );
            }
        }
    }
    let [Some((left, left_name)), Some((right, right_name))] = sides else {
        return Err(ApiError::invalid("both `left` and `right` are required"));
    };
    Ok(DiffInputs {
        left,
        right,
        left_name,
        right_name,
        options: engine::options_text(options.as_deref())?,
    })
}

/// Diffs two inputs sent in the request and returns the Diff Document.
#[utoipa::path(
    post,
    path = "/v1/diff",
    tag = "diffs",
    params(("save" = Option<bool>, Query, description = "Also save the document and both inputs for `GET /v1/diffs/{id}` and for jobs (they expire after the diff retention period)")),
    request_body(
        description = "Two small inputs (each at most the synchronous input limit, 8 MiB by default) and an options object",
        content(
            (DiffRequest = "application/json"),
            (DiffForm = "multipart/form-data"),
        ),
    ),
    responses(
        (status = 200, description = "The canonical Diff Document", body = DiffDocumentSchema, content_type = "application/json"),
        (status = 201, description = "Saved; `Location` is the saved diff", body = DiffDocumentSchema, content_type = "application/json",
            headers(("Location" = String, description = "`/v1/diffs/{id}`"))),
        (status = 400, description = "Invalid request or options", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 413, description = "The body or an input is too large", body = ErrorBody),
        (status = 415, description = "Not JSON or multipart", body = ErrorBody),
        (status = 422, description = "The build cannot compare these inputs", body = ErrorBody),
        (status = 503, description = "Every synchronous diff slot or the shared memory budget stayed taken (`busy`), the request timed out, or a dependency is down", body = ErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn create_diff(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    RawQuery(query): RawQuery,
    request: Request,
) -> Result<Response, ApiError> {
    let save = query_flag(query.as_deref(), "save")?;
    let multipart = match media_type(request.headers()).as_deref() {
        Some("application/json") => false,
        Some("multipart/form-data") => true,
        _ => {
            return Err(ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported-media-type",
                "send application/json or multipart/form-data",
            ));
        }
    };
    // Admitted before the body is read, so bodies in memory are bounded too;
    // the reservation is held until the request is done and the engine has
    // returned. A body without `Content-Length` reserves the body limit.
    let limits = state.config.limits;
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(limits.max_body_bytes)
        .min(limits.max_body_bytes);
    let admitted = state
        .sync
        .admit(
            declared.saturating_mul(SYNC_MEMORY_PER_BODY_BYTE),
            limits.request_timeout / 2,
        )
        .await?;
    let inputs = if multipart {
        multipart_inputs(&state, request).await?
    } else {
        json_inputs(&state, request).await?
    };
    let (left, right) = (inputs.left.clone(), inputs.right.clone());
    let document = engine::run(inputs, Arc::clone(&admitted)).await??;
    if !save {
        return Ok(json_text(StatusCode::OK, document));
    }
    let id = save_diff(&state, &caller, &document, left, right).await?;
    let mut response = json_text(StatusCode::CREATED, document);
    if let Ok(location) = format!("/v1/diffs/{id}").parse() {
        response.headers_mut().insert(header::LOCATION, location);
    }
    Ok(response)
}

async fn save_diff(
    state: &AppState,
    caller: &Caller,
    document: &str,
    left: Bytes,
    right: Bytes,
) -> Result<Uuid, ApiError> {
    let summary = Summary::of(document)?;
    let retention = state.config.retention;
    let blob_expiry = expiry(retention.blob_ttl.max(retention.diff_ttl));
    let doc = store_bytes(
        state,
        Bytes::from(document.to_owned()),
        Some("application/json"),
        blob_expiry,
    )
    .await?;
    let left = store_bytes(state, left, None, blob_expiry).await?;
    let right = store_bytes(state, right, None, blob_expiry).await?;
    let (doc_id, left_id, right_id) = (doc.id.to_hex(), left.id.to_hex(), right.id.to_hex());
    let diff = diffs::create(
        state.db.pool(),
        &NewDiff {
            api_key_id: caller.key_id(),
            document_blob: &doc_id,
            left_blob: Some(&left_id),
            right_blob: Some(&right_id),
            status: &summary.status,
            kind: &summary.kind,
            stats: summary.stats.as_ref(),
            expires_at: expiry(retention.diff_ttl),
        },
    )
    .await?;
    tracing::info!(diff_id = %diff.id, size = doc.size, "diff saved");
    Ok(diff.id)
}

fn diff_not_found() -> ApiError {
    ApiError::not_found("diff-not-found", "no such diff")
}

fn parse_id(text: &str) -> Option<Uuid> {
    Uuid::parse_str(text).ok()
}

/// Returns a saved Diff Document.
#[utoipa::path(
    get,
    path = "/v1/diffs/{id}",
    tag = "diffs",
    params(("id" = Uuid, Path, description = "Saved diff ID")),
    responses(
        (status = 200, description = "The canonical Diff Document, as saved", body = DiffDocumentSchema, content_type = "application/json"),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 404, description = "No such diff, or it has expired", body = ErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn get_diff(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    IdPath(id): IdPath,
) -> Result<Response, ApiError> {
    let id = parse_id(&id).ok_or_else(diff_not_found)?;
    let diff = diffs::get(state.db.pool(), id)
        .await?
        .filter(|d| caller.owns(d.api_key_id))
        .ok_or_else(diff_not_found)?;
    let blob: BlobId = diff
        .document_blob
        .parse()
        .map_err(|_| ApiError::internal())?;
    let (meta, stream) = state.store.get(blob).await?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json".to_owned()),
            (header::CONTENT_LENGTH, meta.size.to_string()),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

/// Deletes a saved diff, the job that produced it, and every blob that
/// nothing else holds any more: no other job or saved diff uses it and no
/// key's upload of it is still live.
#[utoipa::path(
    delete,
    path = "/v1/diffs/{id}",
    tag = "diffs",
    params(("id" = Uuid, Path, description = "Saved diff ID")),
    responses(
        (status = 204, description = "Deleted. A stored object the blob store could not delete at once has no record left and is removed by a later sweep"),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 404, description = "No such diff", body = ErrorBody),
        (status = 503, description = "A dependency is down; nothing was deleted", body = ErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn delete_diff(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    IdPath(id): IdPath,
) -> Result<Response, ApiError> {
    let id = parse_id(&id).ok_or_else(diff_not_found)?;
    diffs::get(state.db.pool(), id)
        .await?
        .filter(|d| caller.owns(d.api_key_id))
        .ok_or_else(diff_not_found)?;
    // On a task of its own: a request that times out or a client that goes
    // away cannot stop the deletion halfway.
    let task = tokio::spawn(remove_diff(state.clone(), id, crate::actor(&caller)));
    let blobs = task.await.map_err(|_| ApiError::internal())??;
    tracing::info!(diff_id = %id, blobs, "diff deleted");
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Deletes the rows in one transaction and commits, then the objects of the
/// blobs whose rows went, while holding their delete locks (ADR 0012).
/// Returns the number of blobs removed.
async fn remove_diff(state: AppState, id: Uuid, actor: Actor) -> Result<usize, ApiError> {
    // Session locks: this connection is closed, never pooled, when dropped.
    let mut conn = state.db.dedicated().await?;
    let candidates = diffs::blobs_of(&mut conn, id)
        .await?
        .ok_or_else(diff_not_found)?;
    blobs::lock_for_delete(&mut conn, &candidates).await?;
    let mut tx = (&mut *conn).begin().await.map_err(DbError::from)?;
    let removed = diffs::delete(&mut tx, id, &candidates)
        .await?
        .ok_or_else(diff_not_found)?;
    let detail = json!({ "blobs": removed.len() });
    let id_text = id.to_string();
    records::record_audit(
        &mut *tx,
        &AuditEvent {
            actor,
            action: "diff.delete",
            target_kind: Some("diff"),
            target_id: Some(&id_text),
            detail: &detail,
        },
    )
    .await?;
    tx.commit().await.map_err(DbError::from)?;
    delete_objects(&state, &removed).await;
    drop(conn);
    Ok(removed.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_decode_with_limits() {
        let input = |text: Option<&str>, b64: Option<&str>| InputBody {
            name: Some("a.txt".to_owned()),
            text: text.map(str::to_owned),
            base64: b64.map(str::to_owned),
        };
        let (bytes, name) = decode("left", input(Some("hi"), None), 10).unwrap();
        assert_eq!(&bytes[..], b"hi");
        assert_eq!(name.as_deref(), Some("a.txt"));
        let (bytes, _) = decode("left", input(None, Some("AAEC")), 10).unwrap();
        assert_eq!(&bytes[..], [0, 1, 2]);
        assert_eq!(
            decode("left", input(Some("x"), Some("AA==")), 10)
                .unwrap_err()
                .code,
            "invalid-request"
        );
        assert_eq!(
            decode("left", input(None, None), 10).unwrap_err().code,
            "invalid-request"
        );
        assert_eq!(
            decode("left", input(None, Some("!!")), 10)
                .unwrap_err()
                .code,
            "invalid-request"
        );
        assert_eq!(
            decode("left", input(Some("12345678901"), None), 10)
                .unwrap_err()
                .code,
            "input-too-large"
        );
        let big = "A".repeat(400);
        assert_eq!(
            decode("left", input(None, Some(&big)), 10)
                .unwrap_err()
                .code,
            "input-too-large"
        );
        let long = InputBody {
            name: Some("n".repeat(1025)),
            text: Some(String::new()),
            base64: None,
        };
        assert_eq!(
            decode("left", long, 10).unwrap_err().code,
            "invalid-request"
        );
    }

    #[test]
    fn input_names_keep_only_their_final_component() {
        let name = |n: &str| input_name("`left.name`", Some(n.to_owned()));
        assert_eq!(name("a.txt").unwrap().as_deref(), Some("a.txt"));
        assert_eq!(
            name("/home/someone/src/a.txt").unwrap().as_deref(),
            Some("a.txt")
        );
        assert_eq!(
            name(r"C:\Users\someone\b.json").unwrap().as_deref(),
            Some("b.json")
        );
        assert_eq!(name("dir/").unwrap(), None);
        assert_eq!(name("").unwrap_err().code, "invalid-request");
        let error = name("a\0.txt").unwrap_err();
        assert_eq!(error.code, "nul-character");
        assert!(!error.message.contains("a.txt"), "{}", error.message);
        assert_eq!(input_name("`left.name`", None).unwrap(), None);
    }
}
