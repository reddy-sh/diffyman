//! PostgreSQL schema, migrations and typed queries for `diffyman-server`
//! ([ADR 0003](../../../docs/adr/0003-postgres-over-sqlite.md),
//! [ADR 0012](../../../docs/adr/0012-local-service-architecture.md)).
//!
//! Every query is checked at compile time by `sqlx` against the schema in
//! `migrations/`, or against the offline data in the workspace's `.sqlx/`
//! when no database is configured (`SQLX_OFFLINE=true`). The database holds
//! metadata and blob IDs only, never file content.
//!
//! Functions that run one statement take any [`sqlx::PgExecutor`] (a pool, a
//! connection or a transaction); those that must run several statements
//! together take a `&mut PgConnection`, usually a transaction, so the caller
//! decides when to commit.

use std::str::FromStr;
use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

pub mod api_keys;
pub mod blobs;
pub mod diffs;
mod error;
pub mod jobs;
pub mod keys;
pub mod records;
pub mod retention;

pub use error::DbError;
pub use sqlx::postgres::{PgConnection, PgPool, Postgres};
pub use sqlx::{Acquire, PgExecutor, Transaction};

/// Result alias for database operations.
pub type Result<T, E = DbError> = std::result::Result<T, E>;

/// The embedded migrations, applied in order by [`Db::migrate`].
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

/// Connection pool settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DbOptions {
    /// Largest number of open connections.
    pub max_connections: u32,
    /// How long to wait for a free connection.
    pub acquire_timeout: Duration,
    /// Server-side limit on every statement.
    pub statement_timeout: Duration,
}

impl Default for DbOptions {
    fn default() -> DbOptions {
        DbOptions {
            max_connections: 10,
            acquire_timeout: Duration::from_secs(5),
            statement_timeout: Duration::from_secs(30),
        }
    }
}

/// A connection pool to the server's database.
#[derive(Clone, Debug)]
pub struct Db {
    pool: PgPool,
}

impl Db {
    /// Connects to `url`. The URL (which holds the password) never appears in
    /// errors.
    pub async fn connect(url: &str, options: DbOptions) -> Result<Db> {
        let connect = PgConnectOptions::from_str(url).map_err(|_| DbError::InvalidUrl)?;
        Db::connect_with(connect, options).await
    }

    /// Connects with prepared connection options.
    pub async fn connect_with(connect: PgConnectOptions, options: DbOptions) -> Result<Db> {
        let timeout_ms = options.statement_timeout.as_millis().to_string();
        let connect = connect.options([("statement_timeout", timeout_ms.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(options.max_connections)
            .acquire_timeout(options.acquire_timeout)
            .connect_with(connect)
            .await
            .map_err(DbError::Unavailable)?;
        Ok(Db { pool })
    }

    /// Wraps an existing pool.
    pub fn from_pool(pool: PgPool) -> Db {
        Db { pool }
    }

    /// The pool, to pass as an executor.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Applies every migration that has not run yet.
    pub async fn migrate(&self) -> Result<()> {
        MIGRATOR.run(&self.pool).await.map_err(DbError::Migrate)
    }

    /// Starts a transaction.
    pub async fn begin(&self) -> Result<Transaction<'static, Postgres>> {
        Ok(self.pool.begin().await?)
    }

    /// A connection for session-level locks ([`blobs::lock_for_delete`]): it
    /// is closed when dropped, never returned to the pool, so its locks end
    /// with it on every path, errors and cancellation included.
    pub async fn dedicated(&self) -> Result<sqlx::pool::PoolConnection<Postgres>> {
        let mut conn = self.pool.acquire().await?;
        conn.close_on_drop();
        Ok(conn)
    }

    /// Checks that the database answers.
    pub async fn ping(&self) -> Result<()> {
        sqlx::query_scalar!("SELECT 1 AS \"one!\"")
            .fetch_one(&self.pool)
            .await?;
        Ok(())
    }

    /// Closes every connection.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

/// Seconds as `f64`, the type `make_interval(secs => ...)` takes.
pub(crate) fn secs(duration: Duration) -> f64 {
    duration.as_secs_f64()
}
