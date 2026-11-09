//! `diffyman-server keys create|list|revoke`.
//!
//! The commands talk to the database directly, with the server's own
//! configuration, and run only for someone who can read the admin token
//! file (mode 0600, written by the first `serve`). There is no HTTP route
//! that creates keys: the API has no admin surface to attack, and the token
//! never travels over the network.

use std::io::Write;

use diffyman_db::api_keys::{self, ApiKey};
use diffyman_db::keys::Secret;
use diffyman_db::records::{self, Actor, AuditEvent};
use diffyman_db::{Db, DbError, DbOptions};
use serde_json::json;
use uuid::Uuid;

use crate::admin_token::{self, TokenError};
use crate::config::Config;
use crate::guard::SCOPES;

/// Errors of the `keys` commands.
#[derive(Debug, thiserror::Error)]
pub enum KeysError {
    /// The admin token file is missing or unusable.
    #[error(
        "{0} (run `diffyman-server serve` once to create it, or set DIFFYMAN_ADMIN_TOKEN_FILE)"
    )]
    Token(#[from] TokenError),
    /// The database failed.
    #[error("{0}")]
    Db(#[from] DbError),
    /// Bad arguments.
    #[error("{0}")]
    Invalid(String),
    /// Writing the output failed.
    #[error("cannot write output: {0}")]
    Io(#[from] std::io::Error),
}

/// Checks the admin token file and connects to the migrated database.
pub async fn open(config: &Config) -> Result<Db, KeysError> {
    admin_token::load(&config.admin_token_file)?;
    let db = Db::connect(
        config.db_url.expose(),
        DbOptions {
            max_connections: 2,
            ..DbOptions::default()
        },
    )
    .await?;
    db.migrate().await?;
    Ok(db)
}

/// Creates a key with `scopes` (every scope when empty) and returns it with
/// its secret, which is shown once and never stored.
pub async fn create(db: &Db, name: &str, scopes: &[String]) -> Result<(ApiKey, Secret), KeysError> {
    if name.is_empty() || name.chars().count() > 100 {
        return Err(KeysError::Invalid(
            "the name must have 1 to 100 characters".to_owned(),
        ));
    }
    if name.contains('\0') {
        return Err(KeysError::Invalid(
            "the name must not contain a NUL character (U+0000)".to_owned(),
        ));
    }
    let mut scopes: Vec<String> = if scopes.is_empty() {
        SCOPES.iter().map(|s| (*s).to_owned()).collect()
    } else {
        scopes.to_vec()
    };
    if let Some(bad) = scopes.iter().find(|s| !SCOPES.contains(&s.as_str())) {
        return Err(KeysError::Invalid(format!(
            "unknown scope `{bad}` (expected {})",
            SCOPES.join(", ")
        )));
    }
    scopes.sort();
    scopes.dedup();
    let (key, secret) = api_keys::create(db.pool(), name, &scopes).await?;
    let id = key.id.to_string();
    let detail = json!({ "scopes": scopes });
    records::record_audit(
        db.pool(),
        &AuditEvent {
            actor: Actor::Admin,
            action: "key.create",
            target_kind: Some("api_key"),
            target_id: Some(&id),
            detail: &detail,
        },
    )
    .await?;
    Ok((key, secret))
}

/// Revokes a key; `false` when it does not exist or was already revoked.
pub async fn revoke(db: &Db, id: Uuid) -> Result<bool, KeysError> {
    let revoked = api_keys::revoke(db.pool(), id).await?;
    if revoked {
        let id = id.to_string();
        records::record_audit(
            db.pool(),
            &AuditEvent {
                actor: Actor::Admin,
                action: "key.revoke",
                target_kind: Some("api_key"),
                target_id: Some(&id),
                detail: &json!({}),
            },
        )
        .await?;
    }
    Ok(revoked)
}

/// Writes every key (never a secret) as a table.
pub fn write_list(keys: &[ApiKey], out: &mut impl Write) -> std::io::Result<()> {
    let time = |t: Option<chrono::DateTime<chrono::Utc>>| {
        t.map_or_else(
            || "-".to_owned(),
            |t| t.format("%Y-%m-%d %H:%M").to_string(),
        )
    };
    writeln!(
        out,
        "{:<36}  {:<13}  {:<17}  {:<16}  {:<16}  {:<16}  NAME",
        "ID", "PREFIX", "SCOPES", "CREATED", "LAST USED", "REVOKED"
    )?;
    for key in keys {
        writeln!(
            out,
            "{:<36}  {:<13}  {:<17}  {:<16}  {:<16}  {:<16}  {}",
            key.id,
            format!("{}…", key.prefix),
            key.scopes.join(","),
            time(Some(key.created_at)),
            time(key.last_used_at),
            time(key.revoked_at),
            key.name.escape_debug()
        )?;
    }
    Ok(())
}
