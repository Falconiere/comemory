//! `PATCH /api/v1/memories/{id}`, `POST /api/v1/memories/{id}/restore`,
//! `POST /api/v1/memories/{id}/references/refresh` (console-api spec §4).
//!
//! All three mutate, so all three take [`guard_mutating`] first (read-only →
//! `405`, permit contention → `503`) and hold the permit across the blocking
//! write. None is confirm-gated: an edit supersedes rather than overwrites, a
//! restore is the inverse of a delete, and a reference refresh only moves
//! version anchors forward — every one of them is reversible, unlike the
//! `DELETE` and `rebuild` surfaces the confirm gate exists for.

use std::time::Instant;

use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::{patch, post};
use axum::{Json, Router};

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, guard_mutating, respond, run_blocking};

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "PATCH",
            path: "/memories/{id}",
            command: "memories.update",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/memories/{id}/restore",
            command: "memories.restore",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/memories/{id}/references/refresh",
            command: "memories.refresh-refs",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/memories/{id}", patch(update))
        .route("/api/v1/memories/{id}/restore", post(restore))
        .route(
            "/api/v1/memories/{id}/references/refresh",
            post(refresh_refs),
        )
}

/// `PATCH /api/v1/memories/{id}` — patch one memory (`api::update`). A
/// frontmatter-only patch keeps the id; a body patch answers with the new id
/// and the old one under `superseded`.
async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<api::update::Request>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("memories.update", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::update::run(&mut ctx, &id, req)
    })
    .await;
    respond("memories.update", result, started)
}

/// `POST /api/v1/memories/{id}/restore` — bring a soft-deleted memory back
/// (`api::restore`). `400` when the id names a live memory, `404` when it is
/// in neither the live tree nor the trash.
async fn restore(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("memories.restore", &state) {
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
    respond("memories.restore", result, started)
}

/// `POST /api/v1/memories/{id}/references/refresh` — re-pin the memory's
/// code references to the current HEAD (`api::refresh_refs`) and answer with
/// the re-classified `code_refs`. Repo roots resolve through this server's
/// `--root <repo>=<path>` overrides first, then `repo_marker.root_path`.
async fn refresh_refs(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("memories.refresh-refs", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::refresh_refs::run(&mut ctx, &id, state.roots())
    })
    .await;
    respond("memories.refresh-refs", result, started)
}

#[cfg(test)]
#[path = "../tests/memories_edit.rs"]
mod tests;
