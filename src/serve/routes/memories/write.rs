//! `POST /api/v1/memories` (`domains::memories::save`), `DELETE /api/v1/memories/{id}`
//! (`domains::memories::delete`, confirm-gated), and `POST /api/v1/feedback`
//! (`domains::learning::feedback`). The first mutating routes in `/api/v1` — every handler
//! here calls [`guard_mutating`] first (read-only → busy ordering, AC-19),
//! holding the returned permit across the blocking write.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{delete, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::domains::learning;
use crate::serve::AppState;
use crate::serve::routes::mutate_response::{
    mutate_blocking, mutate_response, with_ctx, with_lazy_ctx,
};
use crate::serve::routes::{RouteEntry, require_confirm};

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

/// `POST /api/v1/memories` — save a memory (`domains::memories::save`).
async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<crate::domains::memories::save::Request>,
) -> Response {
    let origin = state.http_origin(&headers);
    mutate_blocking(state, "save", move |state| {
        with_lazy_ctx(state, |ctx| {
            ctx.origin = origin;
            crate::domains::memories::save::run(ctx, req, false, None)
        })
    })
    .await
}

/// `?confirm=true` on `DELETE /memories/{id}` — transport-level; not part of
/// `domains::memories::delete::Request` since the CLI has no `--confirm` concept.
#[derive(Deserialize)]
struct ConfirmQuery {
    #[serde(default)]
    confirm: bool,
}

/// `DELETE /api/v1/memories/{id}` — soft-delete (`domains::memories::delete`), confirm
/// gated. `guard_mutating` (read-only/busy) runs before [`require_confirm`]
/// so a read-only server rejects with `405` even without `?confirm=true`
/// (AC-19).
async fn delete_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ConfirmQuery>,
    headers: HeaderMap,
) -> Response {
    let origin = state.http_origin(&headers);
    mutate_blocking(state, "delete", move |state| {
        require_confirm(query.confirm)?;
        with_ctx(state, Some(origin), |ctx| {
            crate::domains::memories::delete::run(ctx, &id)
        })
    })
    .await
}

/// `POST /api/v1/feedback` — record feedback (`domains::learning::feedback`).
async fn feedback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<learning::feedback::Request>,
) -> Response {
    let origin = state.http_origin(&headers);
    mutate_response(state, "feedback", Some(origin), move |ctx| {
        learning::feedback::run(ctx, req)
    })
    .await
}
