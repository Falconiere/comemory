//! `PATCH /api/v1/memories/{id}`, `POST /api/v1/memories/{id}/restore`,
//! `POST /api/v1/memories/{id}/references/refresh` (console-api spec §4).
//!
//! All three mutate, so all three take [`guard_mutating`] first (read-only →
//! `405`, permit contention → `503`) and hold the permit across the blocking
//! write. None is confirm-gated: an edit supersedes rather than overwrites, a
//! restore is the inverse of a delete, and a reference refresh only moves
//! version anchors forward — every one of them is reversible, unlike the
//! `DELETE` and `rebuild` surfaces the confirm gate exists for.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{patch, post};
use axum::{Json, Router};

use crate::serve::AppState;
use crate::serve::routes::RouteEntry;
use crate::serve::routes::mutate_response::{
    mutate_blocking, mutate_response, with_ctx, with_independent_ctx,
};

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

/// `PATCH /api/v1/memories/{id}` — patch one memory (`domains::memories::update`). A
/// frontmatter-only patch keeps the id; a body patch answers with the new id
/// and the old one under `superseded`.
async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<crate::domains::memories::update::Request>,
) -> Response {
    let origin = state.http_origin(&headers);
    mutate_blocking(state, "memories.update", move |state| {
        with_independent_ctx(state, origin, |ctx| {
            crate::domains::memories::update::run(ctx, &id, req)
        })
    })
    .await
}

/// `POST /api/v1/memories/{id}/restore` — bring a soft-deleted memory back
/// (`domains::memories::restore`). `400` when the id names a live memory, `404` when it is
/// in neither the live tree nor the trash.
async fn restore(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let origin = state.http_origin(&headers);
    mutate_response(state, "memories.restore", Some(origin), move |ctx| {
        crate::domains::memories::restore::run(ctx, &id)
    })
    .await
}

/// `POST /api/v1/memories/{id}/references/refresh` — re-pin the memory's
/// code references to the current HEAD (`domains::memories::refresh_refs`) and answer with
/// the re-classified `code_refs`. Repo roots resolve through this server's
/// `--root <repo>=<path>` overrides first, then `repo_marker.root_path`.
async fn refresh_refs(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    mutate_blocking(state, "memories.refresh-refs", move |state| {
        with_ctx(state, None, |ctx| {
            crate::domains::memories::refresh_refs::run(ctx, &id, state.roots())
        })
    })
    .await
}

#[cfg(test)]
#[path = "../tests/memories_edit.rs"]
mod tests;
