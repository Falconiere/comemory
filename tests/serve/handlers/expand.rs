//! Tests for `src/serve/handlers/expand.rs`.

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
async fn expand_known_memory_one_hop() {
    let fx = graph_fixture::build();
    let seed = format!("m:{}", fx.primary_id);
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get(&format!("/api/expand?id={}&depth=1", seed), state).await;
    assert_eq!(status, StatusCode::OK);
    let nodes = body["nodes"].as_array().unwrap();
    assert!(nodes.iter().any(|n| n["id"] == seed));
    assert!(nodes.iter().any(|n| n["kind"] == "Repo"));
}

#[tokio::test]
async fn expand_unknown_id_returns_404() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/expand?id=m:zzzzzzzz&depth=1", state).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn expand_depth_above_three_returns_400() {
    let fx = graph_fixture::build();
    let seed = format!("m:{}", fx.primary_id);
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get(&format!("/api/expand?id={}&depth=99", seed), state).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_param");
}

#[tokio::test]
async fn expand_missing_id_returns_400() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/expand?depth=1", state).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_param");
}
