//! axum router assembly + the request-gating middleware for `comemory serve`.
//!
//! A single `guard` layer fronts every route: it rejects non-loopback `Host`
//! headers (DNS-rebinding defense) and requires the session token on `/` and
//! `/api/*` (via the `X-Comemory-Token` header or a `?token=` query param).
//! Static frontend assets stay ungated — they carry no secrets. No CORS layer
//! is added, so the default is deny (no `Access-Control-Allow-Origin`).

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use crate::serve::{fileio, handlers, security, AppState};

/// Build the application router with the security middleware layered on.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(handlers::index))
        .route("/api/health", get(handlers::health))
        .route("/api/graph", get(handlers::graph))
        .route("/api/file", get(handlers::get_file).put(handlers::put_file))
        .fallback(handlers::static_asset)
        // Lift axum's 2 MiB default body limit to the editor's own cap so a
        // save between 2 MiB and `MAX_FILE_BYTES` reaches the handler (and its
        // friendly error) instead of a generic framework `413`.
        .layer(DefaultBodyLimit::max(fileio::MAX_FILE_BYTES as usize))
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

/// Extract the token from (in order) the `X-Comemory-Token` header, a `?token=`
/// query parameter, or the `comemory_token` cookie. The query form carries the
/// token on the initial browser navigation to `/`; the cookie (set by the `/`
/// handler) re-authenticates a later page reload, since browser navigation
/// cannot send the custom header and the query is stripped from the URL after
/// first load. The token is URL-unreserved (64 hex chars), so no percent
/// decoding is needed for the query/cookie forms.
fn token_from_request(req: &Request) -> Option<String> {
    if let Some(h) = req
        .headers()
        .get("x-comemory-token")
        .and_then(|v| v.to_str().ok())
    {
        return Some(h.to_string());
    }
    if let Some(t) = req.uri().query().and_then(|q| {
        q.split('&')
            .find_map(|pair| pair.strip_prefix("token="))
            .map(str::to_string)
    }) {
        return Some(t);
    }
    let cookies = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())?;
    cookies
        .split(';')
        .find_map(|c| c.trim().strip_prefix("comemory_token="))
        .map(str::to_string)
}
