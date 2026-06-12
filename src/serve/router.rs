//! axum router assembly + the request-gating middleware for `comemory serve`.
//!
//! A single `guard` layer fronts every route: it rejects non-loopback `Host`
//! headers (DNS-rebinding defense) and requires the session token on `/` and
//! `/api/*` (via the `X-Comemory-Token` header or a `?token=` query param).
//! Static frontend assets stay ungated — they carry no secrets. No CORS layer
//! is added, so the default is deny (no `Access-Control-Allow-Origin`).

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use crate::serve::{handlers, security, AppState};

/// Build the application router with the security middleware layered on.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(handlers::index))
        .route("/api/health", get(handlers::health))
        .route("/api/graph", get(handlers::graph))
        .route("/api/file", get(handlers::get_file).put(handlers::put_file))
        .fallback(handlers::static_asset)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

/// Reject non-loopback hosts; require the token on `/` and `/api/*`.
async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !security::host_is_loopback(host) {
        return (StatusCode::FORBIDDEN, "non-loopback Host header rejected").into_response();
    }
    let path = req.uri().path();
    if path == "/" || path.starts_with("/api/") {
        let provided = token_from_request(&req);
        if !security::token_matches(provided.as_deref(), state.token()) {
            return (StatusCode::UNAUTHORIZED, "missing or invalid token").into_response();
        }
    }
    next.run(req).await
}

/// Extract the token from the `X-Comemory-Token` header or a `?token=` query
/// parameter (the latter so the initial browser navigation to `/` carries it).
fn token_from_request(req: &Request) -> Option<String> {
    if let Some(h) = req
        .headers()
        .get("x-comemory-token")
        .and_then(|v| v.to_str().ok())
    {
        return Some(h.to_string());
    }
    let query = req.uri().query()?;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("token="))
        .map(|v| v.to_string())
}
