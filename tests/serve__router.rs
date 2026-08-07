//! In-process coverage of the `guard` middleware assembled by
//! `src/serve/router.rs` (`build_router`): the loopback `Host` check, the
//! token gate on `/` and `/api/*` (header, query, and cookie forms), the
//! `/api/v1/*` enveloped-vs-plain-text rejection split, and the ungated
//! static-asset path. Driven straight through the router via
//! `tower::ServiceExt::oneshot` (`tests/common/serve_state.rs`) so this
//! coverage is actually recorded — the subprocess-based `serve` tests
//! elsewhere in this suite lose it to a SIGKILLed `.profraw`.

use comemory::memory::Kind;

#[path = "common/serve_state.rs"]
mod serve_state;

#[tokio::test]
async fn authenticated_v1_memories_list_returns_a_seeded_memory_as_json() {
    let session = serve_state::session(false);
    serve_state::save(&session, "guard-covered seeded memory", Kind::Note, "demo");
    let resp = serve_state::send(&session, "GET", "/api/v1/memories", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    let content_type = resp
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.starts_with("application/json"),
        "content-type: {content_type}"
    );
    let items = resp.json["data"]["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "body: {}", resp.text);
}

#[tokio::test]
async fn missing_token_on_root_is_401_plain_text() {
    let session = serve_state::session(false);
    let resp =
        serve_state::send_headers(&session, "GET", "/", &[("Host", "127.0.0.1")], None).await;
    assert_eq!(resp.status, 401);
    assert_eq!(resp.json, serde_json::Value::Null, "body: {}", resp.text);
    assert!(
        resp.text.contains("missing or invalid token"),
        "body: {}",
        resp.text
    );
}

#[tokio::test]
async fn missing_token_on_v1_health_is_401_enveloped_json() {
    let session = serve_state::session(false);
    let resp = serve_state::send_headers(
        &session,
        "GET",
        "/api/v1/health",
        &[("Host", "127.0.0.1")],
        None,
    )
    .await;
    assert_eq!(resp.status, 401);
    assert_eq!(
        resp.json["ok"],
        serde_json::json!(false),
        "body: {}",
        resp.text
    );
    assert_eq!(resp.json["error"]["code"], "unauthorized");
    assert_eq!(resp.json["meta"]["command"], "auth");
}

#[tokio::test]
async fn non_loopback_host_on_root_is_403_plain_text() {
    let session = serve_state::session(false);
    let resp = serve_state::send_headers(
        &session,
        "GET",
        "/",
        &[
            ("Host", "evil.example"),
            ("X-Comemory-Token", &session.token),
        ],
        None,
    )
    .await;
    assert_eq!(resp.status, 403);
    assert!(
        resp.text.contains("non-loopback Host header rejected"),
        "body: {}",
        resp.text
    );
}

#[tokio::test]
async fn non_loopback_host_on_v1_is_403_enveloped_json() {
    let session = serve_state::session(false);
    let resp = serve_state::send_headers(
        &session,
        "GET",
        "/api/v1/health",
        &[
            ("Host", "evil.example"),
            ("X-Comemory-Token", &session.token),
        ],
        None,
    )
    .await;
    assert_eq!(resp.status, 403);
    assert_eq!(
        resp.json["ok"],
        serde_json::json!(false),
        "body: {}",
        resp.text
    );
    assert_eq!(resp.json["error"]["code"], "forbidden");
    assert_eq!(resp.json["meta"]["command"], "auth");
}

#[tokio::test]
async fn token_accepted_via_header() {
    let session = serve_state::session(false);
    let resp = serve_state::send(&session, "GET", "/api/v1/health", None).await;
    assert_eq!(resp.status, 200);
    assert_eq!(
        resp.json["ok"],
        serde_json::json!(true),
        "body: {}",
        resp.text
    );
}

#[tokio::test]
async fn token_accepted_via_query_param() {
    let session = serve_state::session(false);
    let path = format!("/api/v1/health?token={}", session.token);
    let resp =
        serve_state::send_headers(&session, "GET", &path, &[("Host", "127.0.0.1")], None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["ok"], serde_json::json!(true));
}

#[tokio::test]
async fn token_accepted_via_cookie() {
    let session = serve_state::session(false);
    let cookie = format!("comemory_token={}", session.token);
    let resp = serve_state::send_headers(
        &session,
        "GET",
        "/api/v1/health",
        &[("Host", "127.0.0.1"), ("Cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["ok"], serde_json::json!(true));
}

#[tokio::test]
async fn static_asset_path_is_ungated_by_the_token_check() {
    let session = serve_state::session(false);
    // No token header at all — a gated path (`/`, `/api/*`) would 401 here.
    let resp = serve_state::send_headers(
        &session,
        "GET",
        "/no-such-asset.js",
        &[("Host", "127.0.0.1")],
        None,
    )
    .await;
    assert_ne!(resp.status, 401, "static assets must not require a token");
    assert_eq!(resp.status, 404, "body: {}", resp.text);
}
