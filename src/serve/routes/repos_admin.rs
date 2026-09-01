//! `POST /api/v1/repos`, `PATCH /api/v1/repos/{name}`,
//! `POST /api/v1/repos/{name}/archive`, `DELETE /api/v1/repos/{name}`
//! (console-api spec §10).
//!
//! The write half of the repository surface; `GET /api/v1/repos`
//! ([`super::repos`]) stays the read half. Every handler here is mutating,
//! so each calls [`guard_mutating`] first (read-only → `405`, contended
//! write permit → `503`), and the destructive one adds [`require_confirm`]
//! after it (AC-19 ordering: read-only outranks a missing confirm).
//!
//! Any `root` a client sends is contained against `AppState::allowed_roots`
//! here, BEFORE `api::repo_admin` writes it, exactly as
//! `POST /api/v1/code/index` contains its `path`.

use std::path::Path;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, State};
use axum::response::Response;
use axum::routing::{delete, post};
use axum::{Json, Router};
use rusqlite::Connection;
use serde::Deserialize;

use crate::api::index_code::IndexMode;
use crate::api::repo_admin::{ArchiveRequest, ConnectRequest, ConnectResponse, PatchRequest};
use crate::api::{self, Ctx};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::{
    RouteEntry, guard_mutating, index_runs, require_confirm, respond, run_blocking,
};
use crate::serve::security;

/// Envelope/route-table command for `POST /repos`.
const CONNECT: &str = "repos.connect";
/// Envelope/route-table command for `PATCH /repos/{name}`.
const PATCH: &str = "repos.patch";
/// Envelope/route-table command for `POST /repos/{name}/archive`.
const ARCHIVE: &str = "repos.archive";
/// Envelope/route-table command for `DELETE /repos/{name}`.
const DISCONNECT: &str = "repos.disconnect";

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "POST",
            path: "/repos",
            command: CONNECT,
            mutating: true,
        },
        RouteEntry {
            method: "PATCH",
            path: "/repos/{name}",
            command: PATCH,
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/repos/{name}/archive",
            command: ARCHIVE,
            mutating: true,
        },
        RouteEntry {
            method: "DELETE",
            path: "/repos/{name}",
            command: DISCONNECT,
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`. `POST /api/v1/repos`
/// merges into the method router [`super::repos`] already mounts `GET` on.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/repos", post(connect_repo))
        .route(
            "/api/v1/repos/{name}",
            delete(disconnect_repo).patch(patch_repo),
        )
        .route("/api/v1/repos/{name}/archive", post(archive_repo))
}

/// `POST /api/v1/repos` — register a working-tree root under a repo label
/// (`api::repo_admin::connect`). With `index_now`, the same `index-code`
/// job `POST /api/v1/index/runs` starts is spawned afterwards (the write
/// permit is already released by then) and its id returned as `job_id`.
async fn connect_repo(State(state): State<AppState>, Json(req): Json<ConnectRequest>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating(CONNECT, &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let write_state = state.clone();
    let connected = run_blocking(move || {
        let _permit = permit;
        let index_now = req.index_now;
        let cfg = write_state.cfg();
        let mut conn = write_state.conn()?;
        let root = contain(&write_state, &conn, &req.root)?;
        let mut ctx = Ctx::borrowed(write_state.paths(), &cfg, &mut conn);
        let resp = api::repo_admin::connect(&mut ctx, ConnectRequest { root, ..req })?;
        Ok((resp, index_now))
    })
    .await;
    let result = match connected {
        Ok((resp, true)) => start_initial_index(&state, resp),
        Ok((resp, false)) => Ok(resp),
        Err(e) => Err(e),
    };
    respond(CONNECT, result, started)
}

/// Spawn the post-connect `index_now` job and record its id on `resp`. A
/// repo that already has a live run is `409 index_running` — the same gate
/// `POST /api/v1/index/runs` applies.
fn start_initial_index(state: &AppState, mut resp: ConnectResponse) -> Result<ConnectResponse> {
    index_runs::refuse_if_running(state, &resp.repo)?;
    let job_id = index_runs::spawn_index_job(
        state,
        resp.repo.clone(),
        resp.root_path.clone(),
        IndexMode::default(),
    )?;
    resp.job_id = Some(job_id);
    Ok(resp)
}

/// `PATCH /api/v1/repos/{name}` — move a repo's root
/// (`api::repo_admin::patch`). Every other field is `501 unsupported`.
async fn patch_repo(
    State(state): State<AppState>,
    UrlPath(name): UrlPath<String>,
    Json(req): Json<PatchRequest>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating(PATCH, &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let root = req
            .root
            .as_deref()
            .map(|root| contain(&state, &conn, root))
            .transpose()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::repo_admin::patch(&mut ctx, &name, PatchRequest { root, ..req })
    })
    .await;
    respond(PATCH, result, started)
}

/// `POST /api/v1/repos/{name}/archive` — flip `repo_marker.archived`
/// (`api::repo_admin::archive`). The body is optional: an empty one
/// archives ([`ArchiveRequest::default`]).
async fn archive_repo(
    State(state): State<AppState>,
    UrlPath(name): UrlPath<String>,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating(ARCHIVE, &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let req = archive_body(&body)?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::repo_admin::archive(&mut ctx, &name, req)
    })
    .await;
    respond(ARCHIVE, result, started)
}

/// Parse the optional archive body. Read as raw [`Bytes`] rather than
/// `Json<_>` so a bodyless `POST` (no `content-type` either) is the
/// documented "archive it" default instead of a `415`.
fn archive_body(body: &Bytes) -> Result<ArchiveRequest> {
    if body.is_empty() {
        return Ok(ArchiveRequest::default());
    }
    serde_json::from_slice(body).map_err(Error::Json)
}

/// `?confirm=true` on `DELETE /repos/{name}` — transport-level, like
/// `DELETE /memories/{id}`'s.
#[derive(Deserialize)]
struct ConfirmQuery {
    #[serde(default)]
    confirm: bool,
}

/// `DELETE /api/v1/repos/{name}?confirm=true` — drop the repo's code index
/// (`api::repo_admin::disconnect`), keeping its memories. Confirm-gated,
/// with [`guard_mutating`] first (AC-19).
async fn disconnect_repo(
    State(state): State<AppState>,
    UrlPath(name): UrlPath<String>,
    Query(query): Query<ConfirmQuery>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating(DISCONNECT, &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        require_confirm(query.confirm)?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::repo_admin::disconnect(&mut ctx, &name)
    })
    .await;
    respond(DISCONNECT, result, started)
}

/// Canonicalize `root` inside an allowed root, as the `String`
/// `api::repo_admin` stores. Shared by the connect and patch handlers so
/// both contain identically.
fn contain(state: &AppState, conn: &Connection, root: &str) -> Result<String> {
    let roots = state.allowed_roots(conn);
    let canonical = security::contain_abs(&roots, Path::new(root))?;
    Ok(canonical.to_string_lossy().into_owned())
}

#[cfg(test)]
#[path = "tests/repos_admin.rs"]
mod tests;
