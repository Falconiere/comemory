#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The five search-shaped routes that answer both `GET` and `POST` — `find`,
//! `code/search`, `search`, `memories/search`, `context` — driven through the
//! real in-process router (`tests/common/serve_state.rs`).
//!
//! These cases were written against the routes while each still had a
//! handler per method, and they must keep passing now that one
//! [`QueryOrJson`](comemory::serve::query_or_json::QueryOrJson) handler
//! serves both: the same query string or JSON body gets the same answer, and
//! a malformed one gets the same rejection status.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::test_common::serve_state::{self, Session};

/// One twin route: its path, and the names its request type gives the query
/// text and the page size (`search` calls them `q` and `limit`).
struct Twin {
    path: &'static str,
    text: &'static str,
    page: &'static str,
}

const TWINS: [Twin; 5] = [
    Twin {
        path: "/api/v1/find",
        text: "query",
        page: "k",
    },
    Twin {
        path: "/api/v1/code/search",
        text: "query",
        page: "k",
    },
    Twin {
        path: "/api/v1/search",
        text: "q",
        page: "limit",
    },
    Twin {
        path: "/api/v1/memories/search",
        text: "query",
        page: "k",
    },
    Twin {
        path: "/api/v1/context",
        text: "query",
        page: "k",
    },
];

/// Send one raw request through the session's router with the loopback
/// `Host` and the session token, so the case controls the content type and
/// the body bytes.
async fn call(
    session: &Session,
    method: &str,
    uri: &str,
    content_type: Option<&str>,
    body: &str,
) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("Host", "127.0.0.1")
        .header("X-Comemory-Token", &session.token);
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    let response = session
        .router
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn enveloped_ok(route: &Twin, method: &str, status: StatusCode, text: &str) {
    assert_eq!(status, StatusCode::OK, "{method} {}: {text}", route.path);
    let envelope: Value = serde_json::from_str(text).unwrap();
    assert_eq!(envelope["ok"], true, "{method} {}: {text}", route.path);
}

#[tokio::test]
async fn every_twin_route_answers_a_get_with_a_query_string() {
    let session = serve_state::session(false);
    for route in &TWINS {
        let uri = format!("{}?{}=pool&{}=2", route.path, route.text, route.page);
        let (status, text) = call(&session, "GET", &uri, None, "").await;
        enveloped_ok(route, "GET", status, &text);
    }
}

#[tokio::test]
async fn every_twin_route_answers_a_post_with_a_json_body() {
    let session = serve_state::session(false);
    for route in &TWINS {
        let body = json!({ route.text: "pool", route.page: 2 }).to_string();
        let (status, text) = call(
            &session,
            "POST",
            route.path,
            Some("application/json"),
            &body,
        )
        .await;
        enveloped_ok(route, "POST", status, &text);
    }
}

#[tokio::test]
async fn every_twin_route_rejects_a_non_numeric_page_size_in_the_query_string() {
    let session = serve_state::session(false);
    for route in &TWINS {
        let uri = format!("{}?{}=pool&{}=abc", route.path, route.text, route.page);
        let (status, text) = call(&session, "GET", &uri, None, "").await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "GET {}: {text}",
            route.path
        );
    }
}

#[tokio::test]
async fn every_twin_route_rejects_truncated_json() {
    let session = serve_state::session(false);
    for route in &TWINS {
        // Cut off after a field every route knows (`query`, aliased as `q` on
        // `search`), so the failure is the syntax error, not an unknown field.
        let (status, text) = call(
            &session,
            "POST",
            route.path,
            Some("application/json"),
            r#"{"query":"pool""#,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "POST {}: {text}",
            route.path
        );
    }
}

#[tokio::test]
async fn every_twin_route_rejects_a_non_json_content_type() {
    let session = serve_state::session(false);
    for route in &TWINS {
        let body = json!({ route.text: "pool" }).to_string();
        let (status, text) = call(&session, "POST", route.path, Some("text/plain"), &body).await;
        assert_eq!(
            status,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "POST {}: {text}",
            route.path
        );
    }
}

#[tokio::test]
async fn every_twin_route_rejects_json_without_its_required_field() {
    let session = serve_state::session(false);
    for route in &TWINS {
        let (status, text) =
            call(&session, "POST", route.path, Some("application/json"), "{}").await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "POST {}: {text}",
            route.path
        );
    }
}
