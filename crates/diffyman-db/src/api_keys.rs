//! API keys: created by the admin, presented by clients.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::keys::{KeyHash, Secret, SecretKind};
use crate::{DbError, Result};

/// An API key as stored, without its secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiKey {
    /// Row ID.
    pub id: Uuid,
    /// The first 12 characters of the key, safe to show.
    pub prefix: String,
    /// A name chosen by the admin.
    pub name: String,
    /// What the key may do, such as `diff` or `diffs:delete`.
    pub scopes: Vec<String>,
    /// When the key was created.
    pub created_at: DateTime<Utc>,
    /// When the key was last used, to the minute.
    pub last_used_at: Option<DateTime<Utc>>,
    /// When the key was revoked.
    pub revoked_at: Option<DateTime<Utc>>,
}

impl ApiKey {
    /// Whether the key carries `scope`.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

/// Creates a key and returns it with its secret, which is never stored and
/// must be shown to the admin once.
pub async fn create(
    executor: impl PgExecutor<'_>,
    name: &str,
    scopes: &[String],
) -> Result<(ApiKey, Secret)> {
    let secret = Secret::generate(SecretKind::ApiKey).map_err(|_| DbError::InvalidValue)?;
    let hash = secret.hash();
    let key = sqlx::query_as!(
        ApiKey,
        "INSERT INTO api_keys (prefix, hash, name, scopes)
         VALUES ($1, $2, $3, $4)
         RETURNING id, prefix, name, scopes, created_at, last_used_at, revoked_at",
        secret.prefix(),
        hash.as_bytes().as_slice(),
        name,
        scopes,
    )
    .fetch_one(executor)
    .await?;
    Ok((key, secret))
}

/// The active key for a presented secret, or `None` when the text is not a
/// key, is unknown or is revoked. The hash comparison takes constant time;
/// `last_used_at` is updated at most once a minute.
pub async fn authenticate(pool: &crate::PgPool, presented: &str) -> Result<Option<ApiKey>> {
    let Some(secret) = Secret::parse(presented, SecretKind::ApiKey) else {
        return Ok(None);
    };
    let rows = sqlx::query!(
        "SELECT id, hash FROM api_keys WHERE prefix = $1 AND revoked_at IS NULL",
        secret.prefix(),
    )
    .fetch_all(pool)
    .await?;
    let mut found = None;
    for row in rows {
        let matches = KeyHash::from_slice(&row.hash).is_some_and(|h| h.verify(&secret));
        if matches {
            found = Some(row.id);
        }
    }
    let Some(id) = found else {
        return Ok(None);
    };
    let key = sqlx::query_as!(
        ApiKey,
        // The update writes only when the stored time is over a minute old;
        // the select reads the row as it was before the update.
        "WITH touched AS (
             UPDATE api_keys SET last_used_at = now()
             WHERE id = $1 AND revoked_at IS NULL
               AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')
             RETURNING id
         )
         SELECT id, prefix, name, scopes, created_at, last_used_at, revoked_at
         FROM api_keys WHERE id = $1 AND revoked_at IS NULL",
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(key)
}

/// Revokes a key. Returns `false` when it does not exist or was already
/// revoked.
pub async fn revoke(executor: impl PgExecutor<'_>, id: Uuid) -> Result<bool> {
    let done = sqlx::query!(
        "UPDATE api_keys SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
        id,
    )
    .execute(executor)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Every key, newest first.
pub async fn list(executor: impl PgExecutor<'_>) -> Result<Vec<ApiKey>> {
    Ok(sqlx::query_as!(
        ApiKey,
        "SELECT id, prefix, name, scopes, created_at, last_used_at, revoked_at
         FROM api_keys ORDER BY id DESC",
    )
    .fetch_all(executor)
    .await?)
}
