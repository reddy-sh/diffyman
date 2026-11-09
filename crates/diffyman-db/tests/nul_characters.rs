//! PostgreSQL cannot store a NUL character (U+0000) in `text` or `jsonb`;
//! such a value is a typed [`DbError::InvalidText`], never a generic database
//! error. Runs against the test database and skips when none is reachable.

#[allow(dead_code)]
mod common;

use chrono::{TimeDelta, Utc};
use diffyman_db::jobs::{self, NewJob};
use diffyman_db::{DbError, PgPool, api_keys, blobs};
use serde_json::{Value, json};

macro_rules! test_db {
    ($name:literal) => {
        match common::test_db($name).await {
            Some(t) => t,
            None => return,
        }
    };
}

async fn create_job(pool: &PgPool, name: &str, options: &Value) -> Result<(), DbError> {
    let blob = "ab".repeat(32);
    let expires_at = Utc::now() + TimeDelta::days(1);
    blobs::upsert(pool, &blob, 1, None, expires_at)
        .await
        .unwrap();
    let new = NewJob {
        api_key_id: None,
        options,
        left_blob: &blob,
        right_blob: &blob,
        left_name: Some(name),
        right_name: None,
        max_attempts: 1,
        expires_at,
    };
    jobs::create(pool, &new).await.map(|_| ())
}

#[tokio::test]
async fn test_nul_in_a_text_column_is_invalid_text() {
    let t = test_db!("test_nul_in_a_text_column_is_invalid_text");
    let error = create_job(t.db.pool(), "a\0.txt", &json!({}))
        .await
        .unwrap_err();
    assert!(matches!(error, DbError::InvalidText), "{error:?}");
    assert_eq!(error.code(), "invalid-text");
    let error = api_keys::create(t.db.pool(), "key\0name", &["read".to_owned()])
        .await
        .unwrap_err();
    assert!(matches!(error, DbError::InvalidText), "{error:?}");
    t.finish().await;
}

#[tokio::test]
async fn test_nul_in_a_jsonb_column_is_invalid_text() {
    let t = test_db!("test_nul_in_a_jsonb_column_is_invalid_text");
    let options = json!({"ignore": {"regex": ["a\u{0}b"]}});
    let error = create_job(t.db.pool(), "a.txt", &options)
        .await
        .unwrap_err();
    assert!(matches!(error, DbError::InvalidText), "{error:?}");
    // The message names no value.
    assert_eq!(error.to_string(), "a text value contains a NUL character");
    t.finish().await;
}
