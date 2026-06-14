//! Async request handlers for the `comemory serve` API. Each handler is thin:
//! it locks the connection only as long as a synchronous query needs, then
//! delegates to [`super::repo_root`] / [`super::fileio`] / [`super::assets`].
//! Authentication and the Host guard are applied upstream by the router
//! middleware, so handlers assume the request is already authorized.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::json;

use crate::cli::graph::{Rel, build_code_graph, build_graph_page};
use crate::prelude::*;
use crate::serve::error::ApiError;
use crate::serve::{AppState, assets, fileio, repo_root};

/// `?id=<file:repo:path>` query for the file endpoints.
#[derive(Deserialize)]
pub struct FileQuery {
    id: String,
}

/// Optional `?rel=…&min_weight=…&limit=…&offset=…` filters for the graph
/// endpoint, mirroring the `comemory graph` CLI flags so the server can filter
/// before sending instead of shipping every edge for the client to hide.
///
/// When **both** `limit` and `offset` are absent the handler returns the full
/// `{nodes, edges}` graph (today's behavior, so the embedded SPA keeps working);
/// when either is present it returns the paginated `GraphPage` envelope.
#[derive(Deserialize)]
pub struct GraphQuery {
    /// `all` (default) | `imports` | `co_changed`.
    rel: Option<String>,
    /// Minimum `co_changed` edge weight; defaults to (and is floored at) 1.
    min_weight: Option<i64>,
    /// Edge-window size. Negative → 400; absent (with `offset`) defaults to 50.
    limit: Option<i64>,
    /// Edges to skip. Negative → 400; absent (with `limit`) defaults to 0.
    offset: Option<i64>,
}

/// `GET /` — the embedded SPA shell with the session token injected.
///
/// Sets the token as an `HttpOnly`, `SameSite=Strict` cookie so a later page
/// reload re-authenticates (browser navigation can't send `X-Comemory-Token`,
/// and the frontend strips the `?token=` query from the URL after first load).
/// `SameSite=Strict` keeps the cookie off cross-site requests, so it adds no
/// CSRF surface; no `Secure` flag because the server is plain http on loopback.
/// `Referrer-Policy: no-referrer` stops the token leaking via `Referer`.
pub async fn index(State(state): State<AppState>) -> std::result::Result<Response, ApiError> {
    let html = assets::index_html_with_token(state.token()).ok_or_else(|| {
        Error::NotFound("frontend not built (web/dist/index.html missing)".into())
    })?;
    let cookie = format!(
        "comemory_token={}; Path=/; HttpOnly; SameSite=Strict",
        state.token()
    );
    Ok((
        [
            (header::REFERRER_POLICY, "no-referrer"),
            (header::SET_COOKIE, cookie.as_str()),
        ],
        Html(html),
    )
        .into_response())
}

/// `GET /api/health` — capability probe used by the frontend to enable or
/// disable the editor's Save action.
pub async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "read_only": state.read_only(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// `GET /api/graph` — the file-level code graph, built by the same routine the
/// static `graph --format html` export uses. Honors optional `rel` /
/// `min_weight` query filters (defaults match the CLI: all relations, weight 1).
///
/// Backward-compatible: with neither `limit` nor `offset` present it returns
/// the full `{nodes, edges}` graph the embedded SPA already consumes. When
/// either is present it paginates the edge dimension and returns the
/// `GraphPage` envelope (`{nodes, edges, limit, offset, total, has_more}`).
/// A negative `limit`/`offset` is rejected with 400, mirroring `min_weight`'s
/// defensive parse.
pub async fn graph(
    State(state): State<AppState>,
    Query(q): Query<GraphQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let rel = match q.rel.as_deref() {
        None | Some("all") => Rel::All,
        Some("imports") => Rel::Imports,
        Some("co_changed") | Some("co-changed") => Rel::CoChanged,
        Some(other) => return Err(Error::BadRequest(format!("unknown rel: {other}")).into()),
    };
    let min_weight = q.min_weight.unwrap_or(1).max(1);
    let conn = state.conn()?;
    if q.limit.is_none() && q.offset.is_none() {
        let graph = build_code_graph(&conn, state.repo(), rel, min_weight)?;
        return Ok(Json(serde_json::to_value(graph).map_err(Error::Json)?));
    }
    let limit = parse_window_param(q.limit, "limit", 50)?;
    let offset = parse_window_param(q.offset, "offset", 0)?;
    let page = build_graph_page(&conn, state.repo(), rel, min_weight, limit, offset)?;
    Ok(Json(serde_json::to_value(page).map_err(Error::Json)?))
}

/// Coerce an optional signed window param into a `usize`, applying `default`
/// when absent and rejecting negatives with a 400 (`BadRequest`) so a bad
/// `?limit=-1` fails loudly rather than silently clamping.
fn parse_window_param(v: Option<i64>, name: &str, default: usize) -> Result<usize> {
    match v {
        None => Ok(default),
        Some(n) if n < 0 => Err(Error::BadRequest(format!("{name} must be >= 0"))),
        Some(n) => Ok(usize::try_from(n).unwrap_or(usize::MAX)),
    }
}

/// `GET /api/file?id=…` — read an indexed source file for the editor.
pub async fn get_file(
    State(state): State<AppState>,
    Query(q): Query<FileQuery>,
) -> std::result::Result<Json<fileio::FileView>, ApiError> {
    let abs = {
        let conn = state.conn()?;
        repo_root::id_to_abs_path(&conn, &q.id, state.roots())?
    };
    let display = repo_root::rel_of(&q.id).unwrap_or(&q.id);
    Ok(Json(fileio::read_file(&abs, display)?))
}

/// `PUT /api/file?id=…` — save an edited source file. `405` when the server is
/// read-only; `409` when the on-disk bytes changed since the client's `GET`
/// (stale `If-Match`).
pub async fn put_file(
    State(state): State<AppState>,
    Query(q): Query<FileQuery>,
    headers: HeaderMap,
    body: String,
) -> std::result::Result<Response, ApiError> {
    if state.read_only() {
        return Ok((StatusCode::METHOD_NOT_ALLOWED, "server is read-only").into_response());
    }
    let abs = {
        let conn = state.conn()?;
        repo_root::id_to_abs_path(&conn, &q.id, state.roots())?
    };
    let if_match = headers
        .get(header::IF_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim_matches('"'));
    match fileio::write_file(&abs, &body, if_match)? {
        fileio::WriteOutcome::Written { blob_oid } => {
            Ok(Json(json!({ "blob_oid": blob_oid })).into_response())
        }
        fileio::WriteOutcome::Conflict { current_oid } => Ok((
            StatusCode::CONFLICT,
            Json(json!({ "current_oid": current_oid })),
        )
            .into_response()),
    }
}

/// Fallback — serve any other embedded frontend asset (Vite's hashed JS/CSS,
/// icons) by its request path. `404` for paths with no embedded file.
///
/// `index.html` (and the bare root, were it ever to fall through here) is
/// redirected to `/` so the only way to a usable SPA shell is the token-gated,
/// token-substituted [`index`] handler — never the raw embedded file with its
/// `__COMEMORY_TOKEN__` sentinel.
pub async fn static_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.is_empty() || path == "index.html" {
        return Redirect::to("/").into_response();
    }
    match assets::asset_bytes(path) {
        Some(bytes) => ([(header::CONTENT_TYPE, assets::mime_for(path))], bytes).into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
