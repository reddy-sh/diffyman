//! Database errors with stable codes.

/// A database error. `Display` never contains the database URL, query
/// parameters or row content; [`DbError::code`] gives a stable code.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The database could not be reached or timed out.
    #[error("database is unavailable")]
    Unavailable(#[source] sqlx::Error),
    /// The database URL could not be parsed.
    #[error("invalid database URL")]
    InvalidUrl,
    /// A row that must exist does not.
    #[error("record not found")]
    NotFound,
    /// A referenced row (such as a blob) does not exist.
    #[error("a referenced record does not exist")]
    MissingReference,
    /// A row with the same unique value exists.
    #[error("a conflicting record exists")]
    Conflict,
    /// The schema rejected a value (a check constraint or a bad type).
    #[error("a value was rejected by the schema")]
    InvalidValue,
    /// A text or JSON value holds a NUL character (U+0000), which
    /// PostgreSQL cannot store in `text` or `jsonb`.
    #[error("a text value contains a NUL character")]
    InvalidText,
    /// A table that only accepts inserts was asked to change.
    #[error("the record is append-only")]
    AppendOnly,
    /// A migration failed.
    #[error("database migration failed")]
    Migrate(#[source] sqlx::migrate::MigrateError),
    /// Any other database error.
    #[error("database error")]
    Other(#[source] sqlx::Error),
}

impl DbError {
    /// A stable kebab-case code.
    pub fn code(&self) -> &'static str {
        match self {
            DbError::Unavailable(_) => "db-unavailable",
            DbError::InvalidUrl => "invalid-db-url",
            DbError::NotFound => "not-found",
            DbError::MissingReference => "missing-reference",
            DbError::Conflict => "conflict",
            DbError::InvalidValue => "invalid-value",
            DbError::InvalidText => "invalid-text",
            DbError::AppendOnly => "append-only",
            DbError::Migrate(_) => "migration-failed",
            DbError::Other(_) => "db-error",
        }
    }
}

impl From<sqlx::Error> for DbError {
    fn from(error: sqlx::Error) -> DbError {
        match &error {
            sqlx::Error::RowNotFound => DbError::NotFound,
            sqlx::Error::PoolTimedOut
            | sqlx::Error::PoolClosed
            | sqlx::Error::Io(_)
            | sqlx::Error::Tls(_) => DbError::Unavailable(error),
            sqlx::Error::Database(db) => match db.code().as_deref() {
                // foreign_key_violation
                Some("23503") => DbError::MissingReference,
                // unique_violation
                Some("23505") => DbError::Conflict,
                // check_violation, not_null_violation, invalid_text_representation
                Some("23514" | "23502" | "22P02") => DbError::InvalidValue,
                // character_not_in_repertoire (a NUL in a `text` value) and
                // untranslatable_character (`\u0000` in a `jsonb` value)
                Some("22021" | "22P05") => DbError::InvalidText,
                // insufficient_privilege, raised by the append-only triggers
                Some("42501") => DbError::AppendOnly,
                // query_canceled (statement_timeout), admin_shutdown, cannot_connect_now
                Some("57014" | "57P01" | "57P03") => DbError::Unavailable(error),
                _ => DbError::Other(error),
            },
            _ => DbError::Other(error),
        }
    }
}
