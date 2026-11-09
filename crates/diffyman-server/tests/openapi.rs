//! `docs/api/openapi.json` must be the document the server serves. When it
//! is not, this test rewrites it and fails, so a stale copy never passes CI
//! and a local run leaves the fresh one ready to commit.

use std::path::Path;

use serde_json::Value;

#[test]
fn committed_openapi_document_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/api/openapi.json");
    let fresh = diffyman_server::api::openapi_json();
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    if committed.replace("\r\n", "\n") != fresh {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &fresh).unwrap();
        panic!("docs/api/openapi.json was out of date and has been rewritten; commit it");
    }
}

#[test]
fn openapi_document_describes_every_route() {
    let doc: Value = serde_json::from_str(&diffyman_server::api::openapi_json()).unwrap();
    assert!(
        doc["openapi"].as_str().unwrap().starts_with("3.1."),
        "{}",
        doc["openapi"]
    );
    let paths = doc["paths"].as_object().unwrap();
    let mut routes: Vec<String> = Vec::new();
    for (path, item) in paths {
        for method in item.as_object().unwrap().keys() {
            routes.push(format!("{} {path}", method.to_uppercase()));
        }
    }
    routes.sort();
    assert_eq!(
        routes,
        [
            "DELETE /v1/diffs/{id}",
            "GET /healthz",
            "GET /openapi.json",
            "GET /v1/diffs/{id}",
            "GET /v1/formats",
            "GET /v1/jobs/{id}",
            "POST /v1/blobs",
            "POST /v1/diff",
            "POST /v1/jobs",
            "POST /v1/jobs/{id}/cancel",
        ]
    );
    // Every /v1 operation needs the bearer key; health and this document do not.
    for (path, item) in paths {
        for operation in item.as_object().unwrap().values() {
            let secured = operation.get("security").is_some();
            assert_eq!(secured, path.starts_with("/v1/"), "{path}");
        }
    }
    assert_eq!(
        doc["components"]["securitySchemes"]["bearer"]["scheme"],
        "bearer"
    );
    assert!(doc["info"].get("contact").is_none());
}
