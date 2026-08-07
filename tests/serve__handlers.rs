//! In-process coverage of the legacy `comemory serve` handlers
//! (`src/serve/handlers.rs`): `GET /api/health`, `GET /api/graph`, `GET
//! /api/file` (containment failure), `GET /` (index + cookie), and the
//! static-asset fallback. Driven through `tower::ServiceExt::oneshot`
//! (`tests/common/serve_state.rs`) so this coverage is actually recorded.

use comemory::memory::Kind;
use comemory::serve::RootOverrides;

#[path = "common/serve_state.rs"]
mod serve_state;

#[tokio::test]
async fn api_health_reports_read_only_and_version() {
    let session = serve_state::session(true);
    let resp = serve_state::send(&session, "GET", "/api/health", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["read_only"], serde_json::json!(true));
    assert_eq!(
        resp.json["version"],
        serde_json::json!(env!("CARGO_PKG_VERSION"))
    );
}

#[tokio::test]
async fn api_graph_returns_the_full_nodes_edges_shape() {
    let session = serve_state::session(false);
    serve_state::save(
        &session,
        "graph handler coverage memory",
        Kind::Note,
        "demo",
    );
    let resp = serve_state::send(&session, "GET", "/api/graph", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert!(resp.json["nodes"].is_array(), "body: {}", resp.text);
    assert!(resp.json["edges"].is_array(), "body: {}", resp.text);
}

#[tokio::test]
async fn api_graph_rejects_a_negative_limit() {
    let session = serve_state::session(false);
    let resp = serve_state::send(&session, "GET", "/api/graph?limit=-1", None).await;
    assert_eq!(resp.status, 400, "body: {}", resp.text);
}

#[tokio::test]
async fn api_file_outside_every_allowed_root_is_forbidden() {
    let home = tempfile::tempdir().expect("root dir");
    let mut roots = RootOverrides::new();
    roots.insert("demo".to_string(), home.path().to_path_buf());
    let session = serve_state::session_with_roots(false, roots);

    // `..` is rejected by `security::resolve_within` before any filesystem
    // access — deterministic regardless of what actually exists on disk.
    let resp = serve_state::send(&session, "GET", "/api/file?id=file:demo:../escape", None).await;
    assert_eq!(resp.status, 403, "body: {}", resp.text);
    assert!(
        resp.text.contains("not allowed in path"),
        "body: {}",
        resp.text
    );
}

#[tokio::test]
async fn api_file_unknown_repo_root_is_bad_request() {
    let session = serve_state::session(false);
    let resp = serve_state::send(&session, "GET", "/api/file?id=file:demo:src/lib.rs", None).await;
    assert_eq!(resp.status, 400, "body: {}", resp.text);
    assert!(
        resp.text.contains("repo root unknown"),
        "body: {}",
        resp.text
    );
}

#[tokio::test]
async fn root_index_serves_html_and_sets_the_token_cookie() {
    let session = serve_state::session(false);
    let resp = serve_state::send(&session, "GET", "/", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    let set_cookie = resp
        .headers
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        set_cookie.contains(&format!("comemory_token={}", session.token)),
        "set-cookie: {set_cookie}"
    );
    assert!(set_cookie.contains("HttpOnly"), "set-cookie: {set_cookie}");
    assert!(
        resp.text.contains(&session.token),
        "index body should carry the substituted token"
    );
}

#[tokio::test]
async fn static_asset_fallback_serves_a_real_embedded_asset() {
    let session = serve_state::session(false);
    // The embedded `web/dist/index.html` redirects to `/` rather than being
    // served raw (only the token-substituted `index` handler may serve it).
    let resp = serve_state::send_headers(
        &session,
        "GET",
        "/index.html",
        &[("Host", "127.0.0.1")],
        None,
    )
    .await;
    assert_eq!(resp.status, 303, "body: {}", resp.text);
    let location = resp
        .headers
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert_eq!(location, "/");
}

#[tokio::test]
async fn static_asset_fallback_404s_an_unknown_path() {
    let session = serve_state::session(false);
    let resp = serve_state::send_headers(
        &session,
        "GET",
        "/no-such-asset.js",
        &[("Host", "127.0.0.1")],
        None,
    )
    .await;
    assert_eq!(resp.status, 404, "body: {}", resp.text);
    assert_eq!(resp.text, "not found");
}
