#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! In-process coverage of the `guard` middleware assembled by
//! `src/serve/router.rs` (`build_router`): the loopback `Host` check, the
//! token gate on `/api/*` (header, bearer, query, and cookie forms), the
//! `/api/v1/*` enveloped-vs-plain-text rejection split, and the unrouted,
//! ungated `/`. Driven straight through the router via
//! `tower::ServiceExt::oneshot` (`tests/common/serve_state.rs`) so this
//! coverage is actually recorded — the subprocess-based `serve` tests
//! elsewhere in this suite lose it to a SIGKILLed `.profraw`.

use comemory::memory::Kind;

use crate::test_common::serve_state;

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

/// An unversioned `/api/*` path is still token-gated, with the plain-text
/// rejection body. The guard matches the whole `/api/*` prefix, not just
/// `/api/v1/*`, and no route is mounted outside the versioned surface — so
/// a request like this one reaches the guard and stops there, answering
/// `401` rather than `404`. That is deliberate: it keeps an unversioned
/// path from ever being an unauthenticated 404 probe.
#[tokio::test]
async fn missing_token_on_a_legacy_api_path_is_401_plain_text() {
    let session = serve_state::session(false);
    let resp = serve_state::send_headers(
        &session,
        "GET",
        "/api/graph",
        &[("Host", "127.0.0.1")],
        None,
    )
    .await;
    assert_eq!(resp.status, 401);
    assert_eq!(resp.json, serde_json::Value::Null, "body: {}", resp.text);
    assert!(
        resp.text.contains("missing or invalid token"),
        "body: {}",
        resp.text
    );
}

/// With the web viewer gone, `/` is neither routed nor token-gated: a bare
/// request is a plain `404`, never a `401`.
#[tokio::test]
async fn root_is_unrouted_and_ungated() {
    let session = serve_state::session(false);
    let resp =
        serve_state::send_headers(&session, "GET", "/", &[("Host", "127.0.0.1")], None).await;
    assert_eq!(resp.status, 404, "body: {}", resp.text);
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
async fn non_loopback_host_on_a_legacy_api_path_is_403_plain_text() {
    let session = serve_state::session(false);
    let resp = serve_state::send_headers(
        &session,
        "GET",
        "/api/graph",
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

/// Console-api spec §1 (AC-1): `Authorization: Bearer <token>` authenticates
/// the versioned surface; a wrong bearer is the enveloped `401`.
#[tokio::test]
async fn token_accepted_via_authorization_bearer() {
    let session = serve_state::session(false);
    let bearer = format!("Bearer {}", session.token);
    let resp = serve_state::send_headers(
        &session,
        "GET",
        "/api/v1/health",
        &[("Host", "127.0.0.1"), ("Authorization", &bearer)],
        None,
    )
    .await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["ok"], serde_json::json!(true));

    let wrong = serve_state::send_headers(
        &session,
        "GET",
        "/api/v1/health",
        &[("Host", "127.0.0.1"), ("Authorization", "Bearer nope")],
        None,
    )
    .await;
    assert_eq!(wrong.status, 401, "body: {}", wrong.text);
    assert_eq!(wrong.json["error"]["code"], "unauthorized");
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

/// Health reports whether an embed command is configured, so a console can
/// tell up front whether `POST /doctor/reembed` will answer `503`.
#[tokio::test]
async fn health_reports_the_embed_command_capability() {
    let session = serve_state::session(false);
    let resp = serve_state::send(&session, "GET", "/api/v1/health", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(
        resp.json["data"]["embed_cmd_configured"],
        serde_json::json!(false)
    );
    assert_eq!(resp.json["data"]["read_only"], serde_json::json!(false));
}

/// AC-18, deterministically: a request body over [`BODY_LIMIT`] is refused
/// `413` by the `DefaultBodyLimit` layer before any handler runs, and the
/// body is plain text, not the `/api/v1` envelope (the layer sits outside
/// the route, so no command name exists to envelope it under). Asserted
/// in-process, where the whole request is handed to the router at once —
/// the real-binary counterpart in `tests/serve__routes__memories__write.rs`
/// cannot pin the status as tightly, because over a socket the rejection
/// races the client's own upload.
#[tokio::test]
async fn a_body_over_the_limit_is_refused_413_before_the_handler() {
    let session = serve_state::session(false);
    let oversized = "a".repeat(comemory::serve::router::BODY_LIMIT + 1);
    let resp = serve_state::send(
        &session,
        "POST",
        "/api/v1/memories",
        Some(serde_json::json!({ "body": oversized })),
    )
    .await;

    assert_eq!(resp.status, 413, "body: {}", resp.text);
    assert_eq!(
        resp.json,
        serde_json::Value::Null,
        "the limit layer answers before the envelope exists: {}",
        resp.text
    );

    let listed = serve_state::send(&session, "GET", "/api/v1/memories", None).await;
    assert_eq!(listed.status, 200, "body: {}", listed.text);
    assert_eq!(
        listed.json["data"]["items"].as_array().map(Vec::len),
        Some(0),
        "nothing over the limit was stored: {}",
        listed.text
    );
}
