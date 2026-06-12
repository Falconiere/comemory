//! Async request handlers for the `comemory serve` API. Each handler is thin:
//! it locks the connection only as long as a synchronous query needs, then
//! delegates to [`super::repo_root`] / [`super::fileio`] / [`super::assets`].
//! Authentication and the Host guard are applied upstream by the router
//! middleware, so handlers assume the request is already authorized.

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::cli::graph::{build_code_graph, Rel};
use crate::prelude::*;
use crate::serve::error::ApiError;
use crate::serve::{assets, fileio, repo_root, AppState};

/// `?id=<file:repo:path>` query for the file endpoints.
#[derive(Deserialize)]
pub struct FileQuery {
    id: String,
}

/// `GET /` — the embedded SPA shell with the session token injected.
pub async fn index(State(state): State<AppState>) -> std::result::Result<Response, ApiError> {
    let html = assets::index_html_with_token(state.token()).ok_or_else(|| {
        Error::NotFound("frontend not built (web/dist/index.html missing)".into())
    })?;
    Ok(Html(html).into_response())
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
/// static `graph --format html` export uses.
pub async fn graph(
    State(state): State<AppState>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let conn = state.conn()?;
    let graph = build_code_graph(&conn, state.repo(), Rel::All, 1)?;
    Ok(Json(serde_json::to_value(graph).map_err(Error::Json)?))
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
pub async fn static_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let key = if path.is_empty() { "index.html" } else { path };
    match assets::asset_bytes(key) {
        Some(bytes) => ([(header::CONTENT_TYPE, assets::mime_for(key))], bytes).into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
