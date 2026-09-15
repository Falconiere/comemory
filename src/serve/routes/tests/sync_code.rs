#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! In-process coverage of `GET /api/v1/sync/code/manifest` and
//! `POST /api/v1/sync/code/import` through the real router.

use serde_json::json;

use crate::test_common::serve_state;

fn one_file_batch() -> serde_json::Value {
    json!({
        "repo": "acme/app",
        "head": "h1",
        "files": [{
            "path": "src/lib.rs",
            "blob_oid": "a".repeat(40),
            "symbols": [{"symbol": "run", "kind": "function", "lang": "rust", "line_start": 1, "line_end": 3}],
            "imports": []
        }]
    })
}

#[tokio::test]
async fn manifest_for_an_unknown_repo_is_empty() {
    let session = serve_state::session(false);
    let resp = serve_state::send(
        &session,
        "GET",
        "/api/v1/sync/code/manifest?repo=acme%2Fapp",
        None,
    )
    .await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["meta"]["command"], "sync.code.manifest");
    assert_eq!(resp.json["data"]["repo"], "acme/app");
    assert!(resp.json["data"]["head"].is_null());
    assert!(resp.json["data"]["files"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn manifest_without_repo_is_a_bad_request() {
    let session = serve_state::session(false);
    let resp = serve_state::send(&session, "GET", "/api/v1/sync/code/manifest", None).await;
    assert_eq!(resp.status, 400, "body: {}", resp.text);
}

#[tokio::test]
async fn manifest_refuses_a_label_the_import_would_refuse() {
    // An untrimmed or colon-bearing label must not be answered "empty" —
    // that would send a client off to push a repo the import then rejects.
    let session = serve_state::session(false);
    for repo in ["%20acme%2Fapp", "acme%3Aapp"] {
        let resp = serve_state::send(
            &session,
            "GET",
            &format!("/api/v1/sync/code/manifest?repo={repo}"),
            None,
        )
        .await;
        assert_eq!(resp.status, 400, "{repo}: {}", resp.text);
        assert!(resp.text.contains("invalid_repo"), "{repo}: {}", resp.text);
    }
}

#[tokio::test]
async fn import_then_manifest_lists_the_file() {
    let session = serve_state::session(false);
    let imported = serve_state::send(
        &session,
        "POST",
        "/api/v1/sync/code/import",
        Some(one_file_batch()),
    )
    .await;
    assert_eq!(imported.status, 200, "body: {}", imported.text);
    assert_eq!(imported.json["meta"]["command"], "sync.code.import");
    assert_eq!(imported.json["data"]["applied"], 1);
    assert_eq!(imported.json["data"]["head"], "h1");
    assert!(
        imported.json["data"]["rejected"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let manifest = serve_state::send(
        &session,
        "GET",
        "/api/v1/sync/code/manifest?repo=acme%2Fapp",
        None,
    )
    .await;
    assert_eq!(manifest.json["data"]["head"], "h1");
    assert_eq!(manifest.json["data"]["files"][0]["path"], "src/lib.rs");

    let nodes =
        serve_state::send(&session, "GET", "/api/v1/graph/nodes?repo=acme%2Fapp", None).await;
    assert_eq!(nodes.status, 200, "body: {}", nodes.text);
    assert_eq!(nodes.json["data"]["items"][0]["label"], "src/lib.rs");
    assert_eq!(nodes.json["data"]["items"][0]["symbols"], 1);
}

#[tokio::test]
async fn import_is_refused_on_a_read_only_server() {
    let session = serve_state::session(true);
    let resp = serve_state::send(
        &session,
        "POST",
        "/api/v1/sync/code/import",
        Some(one_file_batch()),
    )
    .await;
    assert_eq!(resp.status, 405, "body: {}", resp.text);
}

#[tokio::test]
async fn an_invalid_entry_refuses_the_batch_with_a_reason() {
    let session = serve_state::session(false);
    let mut body = one_file_batch();
    body["files"][0]["path"] = json!("../escape.rs");
    let resp = serve_state::send(&session, "POST", "/api/v1/sync/code/import", Some(body)).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["data"]["applied"], 0);
    let reason = resp.json["data"]["rejected"][0]["reason"].as_str().unwrap();
    assert!(reason.starts_with("invalid_path"), "{reason}");
}
