//! axum router assembly + the request-gating middleware for `comemory serve`.
//!
//! A single `guard` layer fronts every route: it rejects non-loopback `Host`
//! headers (DNS-rebinding defense) and requires the session token on
//! `/api/*` (via the `X-Comemory-Token` header, an `Authorization: Bearer`
//! header, a `?token=` query param, or the `comemory_token` cookie). No
//! CORS layer is added, so the default is deny (no
//! `Access-Control-Allow-Origin`). The guard's rejection body is path-aware:
//! `/api/v1/*` gets the enveloped JSON form (`command: "auth"`, AC-11);
//! anything else under `/api/` keeps a plain-text body.

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};

use crate::errors::Error;
use crate::serve::envelope::Envelope;
use crate::serve::{AppState, routes, security};

/// The global request-body ceiling (5 MiB), above axum's 2 MiB default so a
/// large `POST /api/v1/memories` body reaches the handler. Routes that
/// legitimately need more (`POST /api/v1/code/ingest`) layer their own
/// higher limit.
pub const BODY_LIMIT: usize = 5 * 1024 * 1024;

/// Build the application router with the security middleware layered on.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(routes::v1_router(state.clone()))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

/// Reject non-loopback hosts; require the token on `/api/*`.
async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let is_v1 = req.uri().path().starts_with("/api/v1/");
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !security::host_is_loopback(host) {
        return forbidden_response(is_v1);
    }
    if req.uri().path().starts_with("/api/") {
        let provided = token_from_request(&req);
        if !security::token_matches(provided.as_deref(), state.token()) {
            return unauthorized_response(is_v1);
        }
    }
    next.run(req).await
}

/// `403` for a non-loopback `Host` header: enveloped JSON on `/api/v1/*`,
/// plain text everywhere else.
fn forbidden_response(enveloped: bool) -> Response {
    if enveloped {
        return Envelope::err(
            "auth",
            &Error::Forbidden("non-loopback Host header rejected".into()),
            0,
        );
    }
    (StatusCode::FORBIDDEN, "non-loopback Host header rejected").into_response()
}

/// `401` for a missing/invalid session token: enveloped JSON on
/// `/api/v1/*`, plain text everywhere else.
fn unauthorized_response(enveloped: bool) -> Response {
    if enveloped {
        return Envelope::unauthorized("auth");
    }
    (StatusCode::UNAUTHORIZED, "missing or invalid token").into_response()
}

/// Extract the token from (in order) the `X-Comemory-Token` header, an
/// `Authorization: Bearer <token>` header (the console-api spec's §1 form), a
/// `?token=` query parameter (the form a browser `EventSource` can send,
/// since it cannot set headers), or the `comemory_token` cookie. The token
/// is URL-unreserved (64 hex chars), so no percent decoding is needed for
/// the query/cookie forms.
fn token_from_request(req: &Request) -> Option<String> {
    if let Some(h) = req
        .headers()
        .get("x-comemory-token")
        .and_then(|v| v.to_str().ok())
    {
        return Some(h.to_string());
    }
    if let Some(bearer) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        return Some(bearer.trim().to_string());
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

#[cfg(test)]
#[path = "tests/router.rs"]
mod tests;
