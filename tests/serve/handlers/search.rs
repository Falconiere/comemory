use axum::body::Body;
use axum::http::{Request, StatusCode};
use comemory::serve::router::router;
use comemory::serve::state::ServerState;
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
async fn search_matches_tag() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/search?q=database&limit=20", state).await;
    assert_eq!(status, StatusCode::OK);
    let hit = body["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "Tag" && r["label"] == "database");
    assert!(hit.is_some());
}

#[tokio::test]
async fn search_oversize_q_returns_400() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let huge: String = "a".repeat(200);
    let (status, body) = get(&format!("/api/search?q={huge}"), state).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_param");
}

#[tokio::test]
async fn search_missing_q_returns_400() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/search", state).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_param");
}

#[tokio::test]
async fn search_clamps_limit_to_100() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/search?q=a&limit=9999", state).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["results"].as_array().unwrap().len() <= 100);
}
