//! `POST /v1/jobs`, `GET /v1/jobs/{id}`, `POST /v1/jobs/{id}/cancel`.

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use diffyman_db::jobs::{self, Job, JobStatus, NewJob};
use diffyman_db::records::{self, AuditEvent};
use diffyman_db::{DbError, blobs, diffs};
use diffyman_storage::BlobId;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use super::diffs::input_name;
use super::{IdPath, MAX_JOB_REQUEST_BYTES, read_limited};
use crate::content::expiry;
use crate::engine;
use crate::error::{ApiError, ErrorBody, ErrorDetail};
use crate::guard::Caller;
use crate::state::AppState;

/// A diff of two stored inputs, run in the background.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    /// The left (old) input.
    pub left: JobInput,
    /// The right (new) input.
    pub right: JobInput,
    /// An options object (`docs/spec/diff-document.md` §9); `{}` when absent.
    // Kept as its JSON text, for the engine's strict parser.
    #[schema(value_type = Option<Object>)]
    pub options: Option<Box<RawValue>>,
}

/// One input of a job: an uploaded blob (`blob`), or one side of a saved
/// diff (`diff` and `side`), with an optional name.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct JobInput {
    /// A blob ID from this key's `POST /v1/blobs`.
    pub blob: Option<String>,
    /// A saved diff whose input to use.
    pub diff: Option<Uuid>,
    /// Which input of `diff`.
    pub side: Option<Side>,
    /// File name; only its final component is kept.
    pub name: Option<String>,
}

/// A side of a saved diff.
#[derive(Clone, Copy, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Its left input.
    Left,
    /// Its right input.
    Right,
}

/// Where a job is in its life.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    /// Waiting for a worker.
    Queued,
    /// Running.
    Running,
    /// Finished; `result` links to the Diff Document.
    Succeeded,
    /// Finished with `error`.
    Failed,
    /// Cancelled.
    Cancelled,
}

impl From<JobStatus> for JobState {
    fn from(status: JobStatus) -> JobState {
        match status {
            JobStatus::Queued => JobState::Queued,
            JobStatus::Running => JobState::Running,
            JobStatus::Succeeded => JobState::Succeeded,
            JobStatus::Failed => JobState::Failed,
            JobStatus::Cancelled => JobState::Cancelled,
        }
    }
}

/// A job's input as stored.
#[derive(Debug, Serialize, ToSchema)]
pub struct JobSide {
    /// Blob ID.
    pub blob: String,
    /// Name, if given.
    pub name: Option<String>,
}

/// A job.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    /// Job ID (UUIDv7).
    pub id: Uuid,
    /// Status.
    pub status: JobState,
    /// Left input.
    pub left: JobSide,
    /// Right input.
    pub right: JobSide,
    /// Attempts started so far.
    pub attempts: i32,
    /// Attempts allowed (infrastructure errors are retried; input errors are not).
    pub max_attempts: i32,
    /// Whether a cancel was requested while it ran.
    pub cancel_requested: bool,
    /// The last attempt's error.
    pub error: Option<ErrorDetail>,
    /// The saved diff with the result, once succeeded.
    pub diff_id: Option<Uuid>,
    /// Link to the Diff Document, once succeeded.
    #[schema(example = "/v1/diffs/0190f0c4-7b9a-7cc0-8000-000000000000")]
    pub result: Option<String>,
    /// When it was created.
    pub created_at: DateTime<Utc>,
    /// When its first attempt started.
    pub started_at: Option<DateTime<Utc>>,
    /// When it finished.
    pub finished_at: Option<DateTime<Utc>>,
    /// When the job record is removed.
    pub expires_at: DateTime<Utc>,
}

impl From<Job> for JobView {
    fn from(job: Job) -> JobView {
        let error = job.error_code.map(|code| ErrorDetail {
            code,
            message: job.error_message.unwrap_or_default(),
        });
        // A queued job keeps the error of a failed attempt it will retry.
        let error = error.filter(|_| job.status != JobStatus::Succeeded);
        JobView {
            id: job.id,
            status: job.status.into(),
            left: JobSide {
                blob: job.left_blob,
                name: job.left_name,
            },
            right: JobSide {
                blob: job.right_blob,
                name: job.right_name,
            },
            attempts: job.attempts,
            max_attempts: job.max_attempts,
            cancel_requested: job.cancel_requested,
            error,
            diff_id: job.diff_id,
            result: job.diff_id.map(|id| format!("/v1/diffs/{id}")),
            created_at: job.created_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
            expires_at: job.expires_at,
        }
    }
}

/// Whether a string or key anywhere in `value` holds a NUL character, which
/// a `jsonb` column cannot store.
fn has_nul(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains('\0'),
        Value::Array(items) => items.iter().any(has_nul),
        Value::Object(map) => map.iter().any(|(k, v)| k.contains('\0') || has_nul(v)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn blob_not_found() -> ApiError {
    ApiError::not_found("blob-not-found", "no such blob")
}

fn job_not_found() -> ApiError {
    ApiError::not_found("job-not-found", "no such job")
}

/// A resolved input: its blob ID, size and name.
struct Resolved {
    blob: String,
    size: u64,
    name: Option<String>,
}

async fn resolve(
    state: &AppState,
    caller: &Caller,
    which: &str,
    input: JobInput,
) -> Result<Resolved, ApiError> {
    // Stored and shown as the engine records it: the final path component.
    let name = input_name(&format!("`{which}.name`"), input.name)?;
    // An uploaded blob counts only when this key uploaded it (and its grant
    // has not expired): blobs are shared by content, so any other answer
    // would let a key use, or learn of, another key's upload.
    let (blob, granted) = match (input.blob, input.diff, input.side) {
        (Some(blob), None, None) => (blob.parse::<BlobId>()?.to_hex(), true),
        (None, Some(diff), Some(side)) => {
            let diff = diffs::get(state.db.pool(), diff)
                .await?
                .filter(|d| caller.owns(d.api_key_id))
                .ok_or_else(|| ApiError::not_found("diff-not-found", "no such diff"))?;
            match side {
                Side::Left => diff.left_blob,
                Side::Right => diff.right_blob,
            }
            .map(|blob| (blob, false))
            .ok_or_else(|| {
                ApiError::not_found("blob-not-found", "that diff did not keep its inputs")
            })?
        }
        _ => {
            return Err(ApiError::invalid(format!(
                "`{which}` needs `blob`, or `diff` and `side`"
            )));
        }
    };
    let row = if granted {
        blobs::granted(state.db.pool(), &blob, caller.key_id()).await?
    } else {
        blobs::get(state.db.pool(), &blob).await?
    };
    let row = row.ok_or_else(blob_not_found)?;
    Ok(Resolved {
        blob,
        size: u64::try_from(row.size).unwrap_or(u64::MAX),
        name,
    })
}

/// Creates a job that diffs two stored inputs.
#[utoipa::path(
    post,
    path = "/v1/jobs",
    tag = "jobs",
    request_body(content = JobRequest, content_type = "application/json"),
    responses(
        (status = 202, description = "Queued; poll `Location`", body = JobView,
            headers(("Location" = String, description = "`/v1/jobs/{id}`"))),
        (status = 400, description = "Invalid request or options", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 404, description = "A blob this key uploaded, or a diff, does not exist", body = ErrorBody),
        (status = 413, description = "The inputs are over the job memory budget", body = ErrorBody),
        (status = 503, description = "A dependency is down", body = ErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn create_job(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    request: Request,
) -> Result<Response, ApiError> {
    let (parts, body) = request.into_parts();
    let body = read_limited(&parts.headers, body, MAX_JOB_REQUEST_BYTES).await?;
    let request: JobRequest = super::parse_json(&parts.headers, &body)?;
    let options = engine::options_text(request.options.as_deref().map(RawValue::get))?;
    let options: Value = serde_json::from_str(&options).map_err(|_| ApiError::internal())?;
    // Valid for the engine, but the options are stored with the job.
    if has_nul(&options) {
        return Err(ApiError::nul_character("`options`"));
    }
    let left = resolve(&state, &caller, "left", request.left).await?;
    let right = resolve(&state, &caller, "right", request.right).await?;
    let limit = state.config.limits.max_job_input_bytes();
    if left.size.saturating_add(right.size) > limit {
        return Err(ApiError::too_large(
            "job-too-large",
            format!("the inputs are larger than the job limit of {limit} bytes in total"),
        ));
    }
    let config = &state.config;
    let job = jobs::create(
        state.db.pool(),
        &NewJob {
            api_key_id: caller.key_id(),
            options: &options,
            left_blob: &left.blob,
            right_blob: &right.blob,
            left_name: left.name.as_deref(),
            right_name: right.name.as_deref(),
            max_attempts: i32::try_from(config.limits.job_max_attempts).unwrap_or(3),
            expires_at: expiry(config.retention.job_ttl),
        },
    )
    .await
    .map_err(|e| match e {
        DbError::MissingReference => blob_not_found(),
        other => other.into(),
    })?;
    state.jobs_notify.notify_one();
    tracing::info!(job_id = %job.id, size = left.size + right.size, "job queued");
    let mut response = (StatusCode::ACCEPTED, Json(JobView::from(job.clone()))).into_response();
    if let Ok(location) = format!("/v1/jobs/{}", job.id).parse() {
        response.headers_mut().insert(header::LOCATION, location);
    }
    Ok(response)
}

async fn owned_job(state: &AppState, caller: &Caller, id: &str) -> Result<Job, ApiError> {
    let id = Uuid::parse_str(id).map_err(|_| job_not_found())?;
    jobs::get(state.db.pool(), id)
        .await?
        .filter(|j| caller.owns(j.api_key_id))
        .ok_or_else(job_not_found)
}

/// Returns a job's status and, once it succeeded, the link to its result.
#[utoipa::path(
    get,
    path = "/v1/jobs/{id}",
    tag = "jobs",
    params(("id" = Uuid, Path, description = "Job ID")),
    responses(
        (status = 200, description = "The job", body = JobView),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 404, description = "No such job", body = ErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn get_job(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    IdPath(id): IdPath,
) -> Result<Json<JobView>, ApiError> {
    Ok(Json(owned_job(&state, &caller, &id).await?.into()))
}

/// Cancels a job: a queued job at once, a running one at its worker's next
/// heartbeat.
#[utoipa::path(
    post,
    path = "/v1/jobs/{id}/cancel",
    tag = "jobs",
    params(("id" = Uuid, Path, description = "Job ID")),
    responses(
        (status = 200, description = "Cancelled", body = JobView),
        (status = 202, description = "Running; it stops at its next heartbeat", body = JobView),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 404, description = "No such job", body = ErrorBody),
        (status = 409, description = "The job has already finished", body = ErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn cancel_job(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    IdPath(id): IdPath,
) -> Result<Response, ApiError> {
    let job = owned_job(&state, &caller, &id).await?;
    let finished = || {
        ApiError::new(
            StatusCode::CONFLICT,
            "job-finished",
            "the job has already finished",
        )
    };
    if job.status.is_final() {
        return Err(finished());
    }
    let status = jobs::cancel(state.db.pool(), job.id)
        .await?
        .ok_or_else(job_not_found)?;
    if status.is_final() && status != JobStatus::Cancelled {
        return Err(finished());
    }
    let id_text = job.id.to_string();
    let detail = json!({ "status": status.as_str() });
    records::record_audit(
        state.db.pool(),
        &AuditEvent {
            actor: crate::actor(&caller),
            action: "job.cancel",
            target_kind: Some("job"),
            target_id: Some(&id_text),
            detail: &detail,
        },
    )
    .await?;
    let job = jobs::get(state.db.pool(), job.id)
        .await?
        .ok_or_else(job_not_found)?;
    let code = if job.status == JobStatus::Cancelled {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    tracing::info!(job_id = %job.id, outcome = job.status.as_str(), "job cancel requested");
    Ok((code, Json(JobView::from(job))).into_response())
}
