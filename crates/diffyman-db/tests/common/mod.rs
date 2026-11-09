//! Test harness: one fresh schema per test in the `diffyman_test` database.
//!
//! The server is found through `DIFFYMAN_TEST_DB_URL` (a URL to any database
//! on it, usually `postgres`), or else through `DIFFYMAN_PG_PASSWORD` from the
//! environment or the repository's `.env.local`, at the local default
//! `127.0.0.1:5433`. When no server is reachable the test prints why and
//! passes, unless `DIFFYMAN_TEST_DB_REQUIRED=1`, which makes it fail (for CI
//! jobs that run a PostgreSQL service). The main `diffyman` database is never
//! used.

use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use diffyman_db::Db;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, Connection, PgConnection};

const TEST_DATABASE: &str = "diffyman_test";
/// Schemas left behind by crashed runs are dropped after this long.
const STALE_SECS: u64 = 3600;

pub struct TestDb {
    pub db: Db,
    server: PgConnectOptions,
    schema: String,
}

impl TestDb {
    /// Drops the test's schema.
    pub async fn finish(self) {
        // A connection still held by the test would make close() wait forever.
        let closed = tokio::time::timeout(Duration::from_secs(10), self.db.close()).await;
        assert!(closed.is_ok(), "the test still holds a database connection");
        if let Ok(mut conn) = self.server.clone().database(TEST_DATABASE).connect().await {
            let drop = format!("DROP SCHEMA IF EXISTS \"{}\" CASCADE", self.schema);
            let _ = sqlx::raw_sql(sqlx::AssertSqlSafe(drop))
                .execute(&mut conn)
                .await;
            let _ = conn.close().await;
        }
    }
}

fn required() -> bool {
    std::env::var("DIFFYMAN_TEST_DB_REQUIRED").is_ok_and(|v| v == "1")
}

fn skip(test: &str, why: &str) -> Option<TestDb> {
    assert!(!required(), "{test}: DIFFYMAN_TEST_DB_REQUIRED=1 but {why}");
    eprintln!("skipping {test}: {why} (set DIFFYMAN_TEST_DB_URL or run scripts/local-up.sh)");
    None
}

fn password_from_env_file() -> Option<String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env.local");
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == "DIFFYMAN_PG_PASSWORD")
        .map(|(_, value)| value.trim().to_owned())
}

fn server_options() -> Option<PgConnectOptions> {
    if let Ok(url) = std::env::var("DIFFYMAN_TEST_DB_URL") {
        return PgConnectOptions::from_str(&url).ok();
    }
    let password = std::env::var("DIFFYMAN_PG_PASSWORD")
        .ok()
        .or_else(password_from_env_file)?;
    Some(
        PgConnectOptions::new()
            .host("127.0.0.1")
            .port(5433)
            .username("diffyman")
            .password(&password)
            .database("postgres"),
    )
}

async fn ensure_database(server: &PgConnectOptions) -> Result<(), sqlx::Error> {
    let mut conn = server.connect().await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(TEST_DATABASE)
            .fetch_one(&mut conn)
            .await?;
    if !exists {
        let create = format!("CREATE DATABASE {TEST_DATABASE}");
        if let Err(error) = sqlx::raw_sql(sqlx::AssertSqlSafe(create))
            .execute(&mut conn)
            .await
        {
            // Another test created it first: duplicate_database or unique_violation.
            let raced = error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .is_some_and(|code| code == "42P04" || code == "23505");
            if !raced {
                return Err(error);
            }
        }
    }
    conn.close().await
}

async fn drop_stale_schemas(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    let names: Vec<String> =
        sqlx::query_scalar("SELECT nspname::text FROM pg_namespace WHERE nspname LIKE 't\\_%'")
            .fetch_all(&mut *conn)
            .await?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    for name in names {
        let created = name.split('_').nth(1).and_then(|s| s.parse::<u64>().ok());
        if created.is_some_and(|t| now.saturating_sub(t) > STALE_SECS) {
            let drop = format!("DROP SCHEMA IF EXISTS \"{name}\" CASCADE");
            sqlx::raw_sql(sqlx::AssertSqlSafe(drop))
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// A migrated, empty schema for one test, or `None` (after printing why)
/// when no database server is reachable.
pub async fn test_db(test: &str) -> Option<TestDb> {
    let Some(server) = server_options() else {
        return skip(test, "no test database configured");
    };
    let probe = tokio::time::timeout(Duration::from_secs(3), ensure_database(&server)).await;
    match probe {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return skip(test, &format!("database not reachable: {}", short(&error))),
        Err(_) => return skip(test, "database did not answer within 3 s"),
    }
    let test_db = server.clone().database(TEST_DATABASE);
    let mut admin = test_db
        .connect()
        .await
        .expect("connect to the test database");
    drop_stale_schemas(&mut admin)
        .await
        .expect("drop stale test schemas");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut random = [0u8; 6];
    getrandom::fill(&mut random).expect("random schema name");
    let suffix = random.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    });
    let schema = format!("t_{now}_{suffix}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA \"{schema}\"")))
        .execute(&mut admin)
        .await
        .expect("create test schema");
    admin.close().await.expect("close admin connection");

    let options = test_db.options([
        ("search_path", schema.as_str()),
        ("statement_timeout", "30000"),
    ]);
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("connect test pool");
    let db = Db::from_pool(pool);
    db.migrate().await.expect("migrate test schema");
    Some(TestDb { db, server, schema })
}

/// The error without connection details.
fn short(error: &sqlx::Error) -> String {
    match error {
        sqlx::Error::Io(e) => format!("{:?}", e.kind()),
        sqlx::Error::Database(e) => e
            .code()
            .map(std::borrow::Cow::into_owned)
            .unwrap_or_default(),
        sqlx::Error::PoolTimedOut => "timed out".to_owned(),
        _ => "connection failed".to_owned(),
    }
}

/// Opens a second pool on the same schema, for concurrency tests.
pub async fn second_pool(t: &TestDb) -> diffyman_db::PgPool {
    let options = t
        .server
        .clone()
        .database(TEST_DATABASE)
        .options([("search_path", t.schema.as_str())]);
    PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("second pool")
}
