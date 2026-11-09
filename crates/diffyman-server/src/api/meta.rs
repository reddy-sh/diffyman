//! `GET /v1/formats`, `GET /healthz`, `GET /openapi.json` and the fallbacks.

use std::sync::OnceLock;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use diffyman_detect::ExtensionClass;
use diffyman_storage::BlobId;
use serde::Serialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

use super::json_text;
use crate::error::{ApiError, ErrorBody};
use crate::state::AppState;

/// Archive formats of the tree adapter and how each is recognized (spec §19.1).
const ARCHIVES: [(&str, &str); 4] = [
    ("zip", "starts with 50 4B 03 04 or 50 4B 05 06"),
    ("tar", "ustar at bytes 257 to 261 (POSIX and GNU tar)"),
    ("tar.gz", "gzip (1F 8B) around a tar"),
    ("tar.zst", "zstd (28 B5 2F FD) around a tar"),
];

/// What this server compares and its limits.
#[derive(ToSchema)]
#[schema(as = Formats, value_type = Object)]
pub struct FormatsSchema(pub Value);

/// The catalog of `GET /v1/formats`: the kinds, adapters, algorithm IDs and
/// input formats of the engine (as `diffyman formats --json` lists them,
/// without folders, which the server never reads), plus the server's limits.
pub fn catalog(state: &AppState) -> Value {
    let extensions = |class: ExtensionClass| -> Value {
        diffyman_detect::EXTENSIONS
            .iter()
            .filter(|(_, c, _)| *c == class)
            .map(|(ext, _, media)| json!({ "extension": ext, "mediaType": media }))
            .collect()
    };
    let limits = state.config.limits;
    json!({
        "schemaVersion": diffyman::model::SCHEMA_VERSION,
        "engine": { "name": diffyman::ENGINE_NAME, "version": diffyman::ENGINE_VERSION },
        "detect": diffyman_detect::DETECT_ID,
        "kinds": [
            {
                "kind": "text",
                "adapter": "text",
                "algorithms": [
                    diffyman::model::Algorithm::Histogram.id(),
                    diffyman::model::Algorithm::Myers.id(),
                    diffyman::model::Algorithm::Patience.id(),
                ],
                "refine": diffyman_algo::refine::REFINE_ID,
                "moves": diffyman_algo::moves::MOVES_ID,
                "inputs": "UTF-8 text or empty input (a UTF-8 BOM is removed); LF, CRLF and CR line endings",
                "extensions": extensions(ExtensionClass::Text),
            },
            {
                "kind": "binary",
                "adapter": "binary",
                "algorithms": [diffyman_adapter_binary::BINARY_ID],
                "inputs": "any bytes: everything that is not UTF-8 text or an archive pair",
                "extensions": extensions(ExtensionClass::Binary),
            },
            {
                "kind": "tree",
                "adapter": "tree",
                "algorithms": [diffyman_adapter_tree::TREE_ID],
                "inputs": "two archives (the server never reads folders)",
                "archives": ARCHIVES
                    .iter()
                    .map(|(format, magic)| json!({ "format": format, "recognizedBy": magic }))
                    .collect::<Vec<_>>(),
            },
        ],
        "outputFormats": ["json"],
        "limits": {
            "maxBodyBytes": limits.max_body_bytes,
            "maxSyncInputBytes": limits.max_sync_input_bytes,
            "maxBlobBytes": limits.max_blob_bytes,
            "maxJobInputBytes": limits.max_job_input_bytes(),
            "jobTimeoutSecs": limits.job_timeout.as_secs(),
            "requestTimeoutSecs": limits.request_timeout.as_secs(),
        },
    })
}

/// Lists what the server compares and its limits.
#[utoipa::path(
    get,
    path = "/v1/formats",
    tag = "meta",
    responses(
        (status = 200, description = "Kinds, adapters, algorithm IDs, input formats and limits", body = FormatsSchema),
        (status = 401, description = "No valid API key", body = ErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn formats(State(state): State<AppState>) -> Json<Value> {
    Json(catalog(&state))
}

/// Health of the server and its dependencies.
#[derive(Debug, Serialize, ToSchema)]
pub struct Health {
    /// `ok`, or `unavailable` when a dependency is down.
    #[schema(example = "ok")]
    pub status: &'static str,
    /// Each dependency: `ok` or `unavailable`.
    pub checks: HealthChecks,
}

/// Dependency checks.
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthChecks {
    /// The database.
    pub database: &'static str,
    /// The blob store.
    pub storage: &'static str,
}

/// Reports whether the server can reach its database and blob store. Needs
/// no API key.
#[utoipa::path(
    get,
    path = "/healthz",
    tag = "meta",
    responses(
        (status = 200, description = "Healthy", body = Health),
        (status = 503, description = "A dependency is down", body = Health),
    ),
)]
pub async fn healthz(State(state): State<AppState>) -> Response {
    let wait = Duration::from_secs(2);
    let database = matches!(
        tokio::time::timeout(wait, state.db.ping()).await,
        Ok(Ok(()))
    );
    let probe = BlobId::from_digest([0; 32]);
    let storage = matches!(
        tokio::time::timeout(wait, state.store.head(probe)).await,
        Ok(Ok(_))
    );
    let word = |ok: bool| if ok { "ok" } else { "unavailable" };
    let healthy = database && storage;
    let status = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = Health {
        status: word(healthy),
        checks: HealthChecks {
            database: word(database),
            storage: word(storage),
        },
    };
    (status, Json(body)).into_response()
}

/// The OpenAPI 3.1 document of this API. Needs no API key.
#[utoipa::path(
    get,
    path = "/openapi.json",
    tag = "meta",
    responses((status = 200, description = "OpenAPI 3.1 document", content_type = "application/json", body = Object)),
)]
pub async fn openapi() -> Response {
    static TEXT: OnceLock<String> = OnceLock::new();
    let text = TEXT.get_or_init(super::openapi_json);
    json_text(StatusCode::OK, text.clone())
}

/// Unknown routes.
pub async fn not_found() -> ApiError {
    ApiError::not_found("not-found", "no such route")
}

/// Known routes with another method.
pub async fn method_not_allowed() -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method-not-allowed",
        "this route does not take that method",
    )
}
