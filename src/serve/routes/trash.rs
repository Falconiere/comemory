//! `GET /api/v1/trash`, `POST /api/v1/trash/{id}/restore` (console-api spec §9).
//!
//! The trash is its own resource rather than a `memories` sub-resource: its
//! rows are exactly the ones `GET /memories` excludes, and its restore route
//! is a second path onto the same `api::restore::run` that backs
//! `POST /memories/{id}/restore` — one handler body, two addresses, so the
//! console can call whichever reads better in context without a second
//! implementation.

use std::time::Instant;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, post};

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, guard_mutating, respond, run_blocking};

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/trash",
            command: "trash",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/trash/{id}/restore",
            command: "trash.restore",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/trash", get(list))
        .route("/api/v1/trash/{id}/restore", post(restore))
}

/// `GET /api/v1/trash?limit=&offset=` — page soft-deleted memories with the
/// days left before `comemory gc` reaps each one (`api::trash`).
async fn list(State(state): State<AppState>, Query(req): Query<api::trash::Request>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::trash::run(&mut ctx, req)
    })
    .await;
    respond("trash", result, started)
}

/// `POST /api/v1/trash/{id}/restore` — the trash-side address of
/// `api::restore::run`, identical in behavior to
/// `POST /api/v1/memories/{id}/restore`.
async fn restore(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("trash.restore", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::restore::run(&mut ctx, &id)
    })
    .await;
    respond("trash.restore", result, started)
}

#[cfg(test)]
#[path = "tests/trash.rs"]
mod tests;
