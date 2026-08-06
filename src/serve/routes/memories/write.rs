//! `POST /api/v1/memories` (`api::save`), `DELETE /api/v1/memories/{id}`
//! (`api::delete`, confirm-gated), and `POST /api/v1/feedback`
//! (`api::feedback`). The first mutating routes in `/api/v1` — every handler
//! here calls [`guard_mutating`] first (read-only → busy ordering, AC-19),
//! holding the returned permit across the blocking write.

use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{delete, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, guard_mutating, require_confirm, respond, run_blocking};

/// This resource's route-table entries, appended onto [`super::table_entries`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "POST",
            path: "/memories",
            command: "save",
            mutating: true,
        },
        RouteEntry {
            method: "DELETE",
            path: "/memories/{id}",
            command: "delete",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/feedback",
            command: "feedback",
            mutating: true,
        },
    ]
}

/// This module's routes, merged into the `memories` resource router.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/memories", post(save))
        .route("/api/v1/memories/{id}", delete(delete_memory))
        .route("/api/v1/feedback", post(feedback))
}

/// `POST /api/v1/memories` — save a memory (`api::save`).
async fn save(State(state): State<AppState>, Json(req): Json<api::save::Request>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("save", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::save::run(&mut ctx, req)
    })
    .await;
    respond("save", result, started)
}

/// `?confirm=true` on `DELETE /memories/{id}` — transport-level; not part of
/// `api::delete::Request` since the CLI has no `--confirm` concept.
#[derive(Deserialize)]
struct ConfirmQuery {
    #[serde(default)]
    confirm: bool,
}

/// `DELETE /api/v1/memories/{id}` — soft-delete (`api::delete`), confirm
/// gated. `guard_mutating` (read-only/busy) runs before [`require_confirm`]
/// so a read-only server rejects with `405` even without `?confirm=true`
/// (AC-19).
async fn delete_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ConfirmQuery>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("delete", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        require_confirm(query.confirm)?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::delete::run(&mut ctx, &id)
    })
    .await;
    respond("delete", result, started)
}

/// `POST /api/v1/feedback` — record feedback (`api::feedback`).
async fn feedback(
    State(state): State<AppState>,
    Json(req): Json<api::feedback::Request>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("feedback", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::feedback::run(&mut ctx, req)
    })
    .await;
    respond("feedback", result, started)
}
