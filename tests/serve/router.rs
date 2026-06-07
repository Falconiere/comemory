//! Tests for `src/serve/router.rs`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use comemory::serve::router::router;
use comemory::serve::state::ServerState;
use tower::ServiceExt;

use super::graph_fixture;

#[tokio::test]
async fn unknown_api_route_returns_404() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/no-such-endpoint")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_static_path_returns_404() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/no-such-file")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn root_response_has_csp_header() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let csp = resp
        .headers()
        .get(axum::http::header::CONTENT_SECURITY_POLICY)
        .expect("csp present")
        .to_str()
        .unwrap();
    assert!(csp.contains("default-src 'self'"));
}
