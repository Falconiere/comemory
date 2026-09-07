//! `GET /api/v1/sync/{changes,manifest}` and `POST /api/v1/sync/import`
//! (memory-sync design spec).

use std::time::Instant;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::sync::ImportRequest;
use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, guard_mutating, respond, run_blocking};

const DEFAULT_CHANGES_LIMIT: usize = 100;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/sync/changes",
            command: "sync.changes",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/sync/manifest",
            command: "sync.manifest",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/sync/import",
            command: "sync.import",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/sync/changes", get(changes))
        .route("/api/v1/sync/manifest", get(manifest))
        .route("/api/v1/sync/import", post(import))
}

/// Query params for `GET /api/v1/sync/changes`.
#[derive(Debug, Deserialize)]
struct ChangesQuery {
    #[serde(default)]
    since: i64,
    #[serde(default = "default_changes_limit")]
    limit: usize,
}

fn default_changes_limit() -> usize {
    DEFAULT_CHANGES_LIMIT
}

/// `GET /api/v1/sync/changes?since=&limit=` — pull log entries above a cursor.
async fn changes(State(state): State<AppState>, Query(query): Query<ChangesQuery>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::sync::changes::run(&mut ctx, query.since, query.limit)
    })
    .await;
    respond("sync.changes", result, started)
}

/// `GET /api/v1/sync/manifest` — 256-bucket content-hash digest.
async fn manifest(State(state): State<AppState>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::sync::manifest::run(&mut ctx)
    })
    .await;
    respond("sync.manifest", result, started)
}

/// Read optional author override from `X-Comemory-Author`.
fn author_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-comemory-author")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// `POST /api/v1/sync/import` — apply a batch of wire entries.
async fn import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ImportRequest>,
) -> Response {
    let started = Instant::now();
    let author = author_from_headers(&headers);
    let permit = match guard_mutating("sync.import", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::sync::import::run(&mut ctx, req, author.as_deref())
    })
    .await;
    respond("sync.import", result, started)
}

#[cfg(test)]
#[path = "tests/sync.rs"]
mod tests;
