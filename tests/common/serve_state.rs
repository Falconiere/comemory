#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Shared in-process `comemory serve` router harness.
//!
//! Every subprocess-based `serve` test (`spawn_serve` + `child.kill()`)
//! loses coverage: a SIGKILLed process never flushes its `.profraw`, so
//! `serve/router.rs`, `serve/handlers.rs`, and the `/api/v1` route modules
//! record 0% under `cargo llvm-cov` regardless of how thoroughly they are
//! exercised. This module builds the real [`comemory::serve::AppState`] and
//! [`comemory::serve::router::build_router`] router directly inside the
//! test binary and drives requests through `tower::ServiceExt::oneshot` —
//! no socket bind, no subprocess, real coverage.

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request as HttpRequest, StatusCode};
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::serve::{AppState, RootOverrides, ServeOptions};
use comemory::store::connection;
use tempfile::TempDir;
use tower::ServiceExt;

/// One in-process serve session: the temp home dir (kept alive so the
/// backing `comemory.db` file survives for the session's lifetime), the
/// session's bearer token, and the assembled router.
pub struct Session {
    /// The session's data-dir. Also used by [`save`] to open a second,
    /// short-lived connection for seeding — the shared `AppState`
    /// connection is private to the router.
    pub home: TempDir,
    /// The session's `X-Comemory-Token` value.
    pub token: String,
    /// The assembled `axum` router (cheap to `.clone()` per request).
    pub router: Router,
}

/// One HTTP response captured from [`send`]/[`send_headers`]: the status,
/// the response headers, and the body both as raw text and (best-effort)
/// parsed JSON — `Value::Null` when the body is empty or not valid JSON, so
/// callers hitting a plain-text legacy route can still assert on `text`.
pub struct Resp {
    /// The HTTP status code.
    pub status: StatusCode,
    /// The response headers (e.g. `Set-Cookie`, `Location`).
    pub headers: HeaderMap,
    /// Best-effort JSON parse of the body.
    pub json: serde_json::Value,
    /// The raw response body as UTF-8 (lossy).
    pub text: String,
}

/// Build a fresh in-process session over an empty temp data-dir.
/// `read_only` mirrors `comemory serve --read-only`.
pub fn session(read_only: bool) -> Session {
    session_with_roots(read_only, RootOverrides::new())
}

/// Build a fresh in-process session with explicit `--root` overrides — the
/// `GET /api/file` containment tests need a resolvable repo root without
/// registering a real `repo_marker` row.
pub fn session_with_roots(read_only: bool, roots: RootOverrides) -> Session {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(home.path());
    let opts = ServeOptions {
        repo: None,
        port: 0,
        read_only,
        roots,
        open: false,
        cfg: Config::defaults(),
        embed_cmd: None,
        allow_path: Vec::new(),
    };
    let state = AppState::new(&paths, opts).expect("AppState::new");
    let token = state.token().to_string();
    let router = comemory::serve::router::build_router(state);
    Session {
        home,
        token,
        router,
    }
}

/// Save one memory through the real command core (`api::save::run`) against
/// `session`'s data-dir, so seeded rows go through the same
/// markdown+SQLite+edges path a real `comemory save` does. Opens its own
/// short-lived connection (the router's shared connection is private).
pub fn save(session: &Session, body: &str, kind: Kind, repo: &str) {
    let paths = Paths::new(session.home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db for seed save");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let req = api::save::Request {
        body: body.to_string(),
        kind,
        repo: repo.to_string(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    };
    api::save::run(&mut ctx, req, false, None).expect("seed save");
}

/// Send a request through `session.router` with the session token attached
/// (`X-Comemory-Token`) and a loopback `Host: 127.0.0.1` header — the
/// authorized happy path most tests want. `body`, when present, is
/// serialized as the JSON request body.
pub async fn send(
    session: &Session,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Resp {
    let token = session.token.clone();
    send_headers(
        session,
        method,
        path,
        &[("Host", "127.0.0.1"), ("X-Comemory-Token", &token)],
        body,
    )
    .await
}

/// Send a request through `session.router` with exactly `headers` attached
/// (no defaults) — the low-level entry point the `guard` middleware tests
/// use to omit/forge the `Host` or token header.
pub async fn send_headers(
    session: &Session,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<serde_json::Value>,
) -> Resp {
    let mut builder = HttpRequest::builder().method(method).uri(path);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&v).expect("serialize body")))
            .expect("build request"),
        None => builder.body(Body::empty()).expect("build request"),
    };
    let response = session
        .router
        .clone()
        .oneshot(request)
        .await
        .expect("router oneshot");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    Resp {
        status,
        headers,
        json,
        text,
    }
}
