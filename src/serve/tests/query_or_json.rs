#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`QueryOrJson`] against the two extractors it stands in for. One handler
//! behind `get(h).post(h)` must answer exactly what a plain `Query` GET
//! handler and a plain `Json` POST handler answer — success and rejection
//! alike — because merging the twin handlers is only a refactor if no client
//! can tell.
//!
//! Each case is sent to a reference router (two handlers, the stock
//! extractors) and to the single-handler router, and the two answers are
//! compared byte for byte before the status is checked against the value
//! the contract fixes.

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Json, Query};
use axum::http::{Method, Request, StatusCode, header};
use axum::routing::get;
use comemory::serve::query_or_json::QueryOrJson;
use serde::Deserialize;
use serde_json::{Value, json};
use tower::ServiceExt;

/// The request shape of a search route: one required field, one optional
/// numeric one that a query string can get wrong.
#[derive(Deserialize)]
struct Probe {
    query: String,
    #[serde(default)]
    k: Option<usize>,
}

fn echo(probe: Probe) -> Json<Value> {
    Json(json!({ "query": probe.query, "k": probe.k }))
}

async fn reference_get(Query(probe): Query<Probe>) -> Json<Value> {
    echo(probe)
}

async fn reference_post(Json(probe): Json<Probe>) -> Json<Value> {
    echo(probe)
}

async fn single(QueryOrJson(probe): QueryOrJson<Probe>) -> Json<Value> {
    echo(probe)
}

/// Small enough that a long query overflows it.
const BODY_LIMIT: usize = 64;

fn reference_router() -> Router {
    Router::new()
        .route("/p", get(reference_get).post(reference_post))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
}

fn single_router() -> Router {
    Router::new()
        .route("/p", get(single).post(single))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
}

/// What a client sees: status, content type, and the body bytes.
type Answer = (StatusCode, Option<String>, Vec<u8>);

async fn answer(
    router: Router,
    method: Method,
    uri: &str,
    content_type: Option<&str>,
    body: &str,
) -> Answer {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    let request = builder.body(Body::from(body.to_string())).unwrap();
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, content_type, bytes)
}

/// Send one case to both routers, require identical answers, and return the
/// shared answer for the case's own assertions.
async fn both(method: Method, uri: &str, content_type: Option<&str>, body: &str) -> Answer {
    let expected = answer(reference_router(), method.clone(), uri, content_type, body).await;
    let actual = answer(single_router(), method, uri, content_type, body).await;
    assert_eq!(
        actual,
        expected,
        "single handler diverged from the stock extractors for {uri}: {} vs {}",
        String::from_utf8_lossy(&actual.2),
        String::from_utf8_lossy(&expected.2),
    );
    actual
}

fn json_body(answer: &Answer) -> Value {
    serde_json::from_slice(&answer.2).unwrap()
}

#[tokio::test]
async fn a_get_reads_its_fields_from_the_query_string() {
    let got = both(Method::GET, "/p?query=pool&k=2", None, "").await;
    assert_eq!(got.0, StatusCode::OK);
    assert_eq!(json_body(&got), json!({ "query": "pool", "k": 2 }));
}

#[tokio::test]
async fn a_get_may_leave_the_optional_field_out() {
    let got = both(Method::GET, "/p?query=pool", None, "").await;
    assert_eq!(got.0, StatusCode::OK);
    assert_eq!(json_body(&got), json!({ "query": "pool", "k": null }));
}

#[tokio::test]
async fn a_post_reads_its_fields_from_the_json_body() {
    let got = both(
        Method::POST,
        "/p",
        Some("application/json"),
        r#"{"query":"pool","k":2}"#,
    )
    .await;
    assert_eq!(got.0, StatusCode::OK);
    assert_eq!(json_body(&got), json!({ "query": "pool", "k": 2 }));
}

#[tokio::test]
async fn a_get_ignores_a_body_it_was_sent() {
    let got = both(
        Method::GET,
        "/p?query=from-query",
        Some("application/json"),
        r#"{"query":"from-body"}"#,
    )
    .await;
    assert_eq!(json_body(&got)["query"], "from-query");
}

#[tokio::test]
async fn a_post_ignores_a_query_string_it_was_sent() {
    let got = both(
        Method::POST,
        "/p?query=from-query",
        Some("application/json"),
        r#"{"query":"from-body"}"#,
    )
    .await;
    assert_eq!(json_body(&got)["query"], "from-body");
}

#[tokio::test]
async fn a_head_takes_the_query_path_like_the_get_it_shadows() {
    let got = both(Method::HEAD, "/p?query=pool", None, "").await;
    assert_eq!(got.0, StatusCode::OK);
    assert!(got.2.is_empty(), "a HEAD response carries no body");
}

#[tokio::test]
async fn a_non_numeric_page_size_in_the_query_string_is_the_stock_query_rejection() {
    let got = both(Method::GET, "/p?query=pool&k=abc", None, "").await;
    assert_eq!(got.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_get_without_the_required_field_is_the_stock_query_rejection() {
    let got = both(Method::GET, "/p?k=2", None, "").await;
    assert_eq!(got.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn truncated_json_is_the_stock_json_rejection() {
    let got = both(Method::POST, "/p", Some("application/json"), r#"{"query":"#).await;
    assert_eq!(got.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_non_json_content_type_is_the_stock_json_rejection() {
    let got = both(
        Method::POST,
        "/p",
        Some("text/plain"),
        r#"{"query":"pool"}"#,
    )
    .await;
    assert_eq!(got.0, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn json_missing_the_required_field_is_the_stock_json_rejection() {
    let got = both(Method::POST, "/p", Some("application/json"), "{}").await;
    assert_eq!(got.0, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn json_with_a_wrongly_typed_field_is_the_stock_json_rejection() {
    let got = both(
        Method::POST,
        "/p",
        Some("application/json"),
        r#"{"query":"pool","k":"two"}"#,
    )
    .await;
    assert_eq!(got.0, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_body_over_the_limit_is_the_stock_payload_too_large_rejection() {
    let long = format!(r#"{{"query":"{}"}}"#, "x".repeat(BODY_LIMIT));
    let got = both(Method::POST, "/p", Some("application/json"), &long).await;
    assert_eq!(got.0, StatusCode::PAYLOAD_TOO_LARGE);
}
