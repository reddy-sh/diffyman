//! Append-only usage and audit records. The database rejects any update,
//! delete or truncate of either table.

use std::fmt;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::Result;

/// One API call or job, for usage accounting.
#[derive(Clone, Debug)]
pub struct Usage<'a> {
    /// The key that made the call.
    pub api_key_id: Option<Uuid>,
    /// Dotted lowercase name, such as `diff.sync` or `blobs.put`.
    pub operation: &'a str,
    /// `ok` or a stable error code.
    pub outcome: &'a str,
    /// Bytes received.
    pub bytes_in: i64,
    /// Bytes sent.
    pub bytes_out: i64,
    /// Wall time in milliseconds.
    pub duration_ms: i64,
}

/// Appends a usage record.
pub async fn record_usage(executor: impl PgExecutor<'_>, usage: &Usage<'_>) -> Result<()> {
    sqlx::query!(
        "INSERT INTO usage (api_key_id, operation, outcome, bytes_in, bytes_out, duration_ms)
         VALUES ($1, $2, $3, $4, $5, $6)",
        usage.api_key_id,
        usage.operation,
        usage.outcome,
        usage.bytes_in,
        usage.bytes_out,
        usage.duration_ms,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Usage of one key since a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsageTotals {
    /// Number of records.
    pub calls: i64,
    /// Sum of `bytes_in`.
    pub bytes_in: i64,
    /// Sum of `bytes_out`.
    pub bytes_out: i64,
}

/// Totals for `api_key_id` since `since`.
pub async fn usage_totals(
    executor: impl PgExecutor<'_>,
    api_key_id: Uuid,
    since: DateTime<Utc>,
) -> Result<UsageTotals> {
    Ok(sqlx::query_as!(
        UsageTotals,
        "SELECT count(*) AS \"calls!\",
                COALESCE(sum(bytes_in), 0)::bigint AS \"bytes_in!\",
                COALESCE(sum(bytes_out), 0)::bigint AS \"bytes_out!\"
         FROM usage WHERE api_key_id = $1 AND at >= $2",
        api_key_id,
        since,
    )
    .fetch_one(executor)
    .await?)
}

/// Who did something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    /// The holder of the admin token.
    Admin,
    /// The server itself, such as the sweeper.
    System,
    /// An API key.
    Key(Uuid),
}

impl fmt::Display for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Actor::Admin => f.write_str("admin"),
            Actor::System => f.write_str("system"),
            Actor::Key(id) => write!(f, "key:{id}"),
        }
    }
}

/// An audit event. `detail` holds IDs, sizes and counts only, never file
/// content, names or paths.
#[derive(Clone, Debug)]
pub struct AuditEvent<'a> {
    /// Who did it.
    pub actor: Actor,
    /// Dotted lowercase name, such as `api_key.create` or `diff.delete`.
    pub action: &'a str,
    /// Kind of record affected, such as `api_key` or `diff`.
    pub target_kind: Option<&'a str>,
    /// ID of the record affected.
    pub target_id: Option<&'a str>,
    /// Extra facts (a JSON object).
    pub detail: &'a Value,
}

/// Appends an audit record.
pub async fn record_audit(executor: impl PgExecutor<'_>, event: &AuditEvent<'_>) -> Result<()> {
    sqlx::query!(
        "INSERT INTO audit (actor, action, target_kind, target_id, detail)
         VALUES ($1, $2, $3, $4, $5)",
        event.actor.to_string(),
        event.action,
        event.target_kind,
        event.target_id,
        event.detail,
    )
    .execute(executor)
    .await?;
    Ok(())
}
