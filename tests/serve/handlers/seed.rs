//! Tests for `src/serve/handlers/seed.rs`.

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
async fn seed_memory_layer_returns_three_memories() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/seed?layer=memory", state).await;
    assert_eq!(status, StatusCode::OK);
    let memories = body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["kind"] == "Memory")
        .count();
    assert_eq!(memories, 3);
    assert!(
        body["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["kind"] != "File" && n["kind"] != "Symbol"),
        "memory layer should not include File/Symbol"
    );
}

#[tokio::test]
async fn seed_all_includes_code_layer() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/seed?layer=all", state).await;
    assert_eq!(status, StatusCode::OK);
    let has_file = body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["kind"] == "File");
    assert!(has_file);
}

#[tokio::test]
async fn seed_invalid_layer_returns_400() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/seed?layer=banana", state).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_param");
}
