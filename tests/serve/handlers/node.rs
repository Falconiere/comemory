//! Tests for `src/serve/handlers/node.rs`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use qwick_memory::serve::router::router;
use qwick_memory::serve::state::ServerState;
use tower::ServiceExt;

use super::graph_fixture;

async fn get(uri: &str, state: ServerState) -> (StatusCode, serde_json::Value) {
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn node_detail_memory_has_body_and_edges() {
    let fx = graph_fixture::build();
    let seed = format!("m:{}", fx.primary_id);
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get(&format!("/api/node/{}", seed), state).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["node"]["kind"], "Memory");
    assert!(body["memory_body"].is_string());
    assert!(body["frontmatter"].is_object());
    let outbound_kinds: Vec<String> = body["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["edge_kind"].as_str().unwrap().to_string())
        .collect();
    assert!(outbound_kinds.iter().any(|k| k == "InRepo"));
}

#[tokio::test]
async fn node_detail_repo_has_no_body() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/node/r:qwick-backend", state).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["node"]["kind"], "Repo");
    assert!(body.get("memory_body").is_none());
    assert!(body.get("frontmatter").is_none());
}

#[tokio::test]
async fn node_detail_unknown_returns_404() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/node/m:zzzzzzzz", state).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}
