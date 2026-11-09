//! Documented input rules (`docs/api/README.md`): input names keep only
//! their final path component, a NUL character in a name or in a job's
//! options is refused before any query with `400 nul-character`, options
//! that give a key twice are `400 invalid-options` in every request form (as
//! with the CLI), and an invalid JSON body or invalid options are described
//! without quoting them.
//!
//! The NUL and JSON tests run on a server whose database is unreachable, so
//! a request that got as far as a query would answer `503 db-unavailable`.
//! The name test needs the test database and skips without one.

#[allow(dead_code)]
#[path = "../../diffyman-db/tests/common/mod.rs"]
mod common;
mod support;

use std::fmt::Write as _;

use axum::http::{Method, StatusCode};
use serde_json::json;
use support::{get, new_key, offline, online, post_json, upload};

const BLOB: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const DIFF: &str = "0190f0c4-7b9a-7cc0-8000-000000000000";

#[tokio::test]
async fn test_job_input_names_keep_only_the_final_path_component() {
    let Some(t) = common::test_db("test_job_input_names_keep_only_the_final_path_component").await
    else {
        return;
    };
    let server = online(t.db.clone(), support::memory_store(), &[]);
    let app = &server.app;
    let key = new_key(&server.state).await;
    let left = upload(app, &key, b"one\n").await;
    let right = upload(app, &key, b"two\n").await;
    let body = json!({
        "left": {"blob": left, "name": "/home/someone/projects/report/old.txt"},
        "right": {"blob": right, "name": r"C:\Users\someone\new.txt"},
    });
    let created = post_json(app, "/v1/jobs", Some(&key), &body).await;
    assert_eq!(created.status, StatusCode::ACCEPTED, "{}", created.text());
    let job = created.json();
    assert_eq!(job["left"]["name"], "old.txt");
    assert_eq!(job["right"]["name"], "new.txt");
    assert!(!created.text().contains("someone"), "{}", created.text());

    // The stored record is the same, so the job's view never shows the path.
    let id = job["id"].as_str().unwrap();
    let fetched = get(app, &format!("/v1/jobs/{id}"), Some(&key)).await;
    assert_eq!(fetched.json()["left"]["name"], "old.txt");
    let stored = diffyman_db::jobs::get(t.db.pool(), id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.left_name.as_deref(), Some("old.txt"));
    assert_eq!(stored.right_name.as_deref(), Some("new.txt"));
    drop(server);
    t.finish().await;
}

#[tokio::test]
async fn test_nul_in_a_job_input_name_is_rejected_before_any_query() {
    let server = offline(&[]);
    let body = json!({
        "left": {"blob": BLOB, "name": "a\u{0}.txt"},
        "right": {"blob": BLOB},
    });
    let reply = post_json(&server.app, "/v1/jobs", None, &body).await;
    assert_eq!(
        (reply.status, reply.code().as_str()),
        (StatusCode::BAD_REQUEST, "nul-character"),
        "{}",
        reply.text()
    );
    assert!(
        reply.json()["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("`left.name`")
    );
}

#[tokio::test]
async fn test_nul_in_job_options_is_rejected_before_any_query() {
    let server = offline(&[]);
    let body = json!({
        "left": {"blob": BLOB},
        "right": {"blob": BLOB},
        "options": {"ignore": {"regex": ["id=\u{0}"]}},
    });
    let reply = post_json(&server.app, "/v1/jobs", None, &body).await;
    assert_eq!(
        (reply.status, reply.code().as_str()),
        (StatusCode::BAD_REQUEST, "nul-character"),
        "{}",
        reply.text()
    );
}

#[tokio::test]
async fn test_nul_in_a_synchronous_input_name_is_rejected() {
    let server = offline(&[]);
    let body = json!({
        "left": {"name": "a\u{0}.txt", "text": "a\n"},
        "right": {"text": "b\n"},
    });
    let reply = post_json(&server.app, "/v1/diff", None, &body).await;
    assert_eq!(reply.code(), "nul-character", "{}", reply.text());
}

fn assert_invalid_options(reply: &support::Reply, message: &str, what: &str) {
    assert_eq!(
        (reply.status, reply.code().as_str()),
        (StatusCode::BAD_REQUEST, "invalid-options"),
        "{what}: {}",
        reply.text()
    );
    assert_eq!(reply.json()["error"]["message"], message, "{what}");
}

#[tokio::test]
async fn test_options_that_repeat_a_key_are_refused_in_every_request_form() {
    let server = offline(&[]);
    let app = &server.app;
    let repeated = message_of(r#"{"context": 1, "context": 2}"#);
    for options in [
        r#"{"context": 1, "context": 2}"#,
        r#"{"context": 3, "context": 3}"#,
        r#"{"ignore": {"case": true, "case": false}}"#,
        r#"{"tree": {"maxDepth": 1, "maxDepth": 2}}"#,
    ] {
        let diff = format!(
            r#"{{"left": {{"text": "a"}}, "right": {{"text": "b"}}, "options": {options}}}"#
        );
        let reply = send_json(app, "/v1/diff", diff).await;
        assert_invalid_options(&reply, &repeated, &format!("diff {options}"));
        // Refused before any query: the database is unreachable.
        let job = format!(
            r#"{{"left": {{"blob": "{BLOB}"}}, "right": {{"blob": "{BLOB}"}}, "options": {options}}}"#
        );
        let reply = send_json(app, "/v1/jobs", job).await;
        assert_invalid_options(&reply, &repeated, &format!("job {options}"));
        let (form, content_type) = multipart(options);
        let reply = support::send(app, Method::POST, "/v1/diff", None, &content_type, form).await;
        assert_invalid_options(&reply, &repeated, &format!("multipart {options}"));
    }
    // A multipart options field that is not JSON.
    let (form, content_type) = multipart("{context");
    let reply = support::send(app, Method::POST, "/v1/diff", None, &content_type, form).await;
    assert_invalid_options(
        &reply,
        "invalid options: the options are not valid JSON",
        "not JSON",
    );
}

#[tokio::test]
async fn test_invalid_options_messages_never_quote_the_request() {
    let server = offline(&[]);
    let marker = "SECRET-MARKER-3b7d";
    let invalid = message_of(r#"{"nope": 1}"#);
    for options in [
        format!(r#"{{"{marker}": 1}}"#),
        format!(r#"{{"granularity": "{marker}"}}"#),
        format!(r#"{{"context": "{marker}"}}"#),
        format!(r#"{{"ignore": {{"{marker}": true}}}}"#),
        format!(r#"{{"ignore": {{"regex": "{marker}"}}}}"#),
        format!(r#"["{marker}"]"#),
    ] {
        let body = format!(
            r#"{{"left": {{"text": "a"}}, "right": {{"text": "b"}}, "options": {options}}}"#
        );
        let reply = send_json(&server.app, "/v1/diff", body).await;
        assert_invalid_options(&reply, &invalid, &options);
        assert!(!reply.text().contains(marker), "{}", reply.text());
    }
    let body = format!(
        r#"{{"left": {{"text": "a"}}, "right": {{"text": "b"}}, "options": {{"{marker}": 1, "{marker}": 2}}}}"#
    );
    let reply = send_json(&server.app, "/v1/diff", body).await;
    assert!(!reply.text().contains(marker), "{}", reply.text());
}

/// The server's message for options the engine refuses like `text`.
fn message_of(text: &str) -> String {
    let error = diffyman::parse_options(text).unwrap_err();
    let error = diffyman_server::error::ApiError::from(error);
    error.message
}

async fn send_json(app: &axum::Router, uri: &str, body: String) -> support::Reply {
    support::send(
        app,
        Method::POST,
        uri,
        None,
        "application/json",
        body.into_bytes(),
    )
    .await
}

fn multipart(options: &str) -> (Vec<u8>, String) {
    let boundary = "input-rules-4c2a";
    let mut form = String::new();
    for (name, extra, data) in [
        ("left", "; filename=\"a.txt\"", "a\n"),
        ("right", "; filename=\"b.txt\"", "b\n"),
        ("options", "", options),
    ] {
        let _ = write!(
            form,
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"{extra}\r\n\r\n{data}\r\n"
        );
    }
    let _ = write!(form, "--{boundary}--\r\n");
    (
        form.into_bytes(),
        format!("multipart/form-data; boundary={boundary}"),
    )
}

#[tokio::test]
async fn test_invalid_json_body_error_does_not_quote_the_body() {
    let server = offline(&[]);
    let marker = "SECRET-MARKER-91c2";
    let cases = [
        ("/v1/diff", json!({"left": marker, "right": {"text": "b"}})),
        (
            "/v1/diff",
            json!({"left": {"text": "a", marker: "x"}, "right": {"text": "b"}}),
        ),
        (
            "/v1/jobs",
            json!({"left": {"diff": DIFF, "side": marker}, "right": {"blob": BLOB}}),
        ),
    ];
    for (uri, body) in cases {
        let reply = post_json(&server.app, uri, None, &body).await;
        assert_eq!(
            (reply.status, reply.code().as_str()),
            (StatusCode::BAD_REQUEST, "invalid-request"),
            "{}",
            reply.text()
        );
        let message = reply.json()["error"]["message"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(!reply.text().contains(marker), "{uri}: {message}");
        assert!(
            message.starts_with("invalid JSON body: data error at line 1, column "),
            "{uri}: {message}"
        );
    }
}
