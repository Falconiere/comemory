//! `GET /api/v1/memories` (`api::list`) and `GET /api/v1/memories/{id}`
//! (`api::show` — the same middle `comemory show --json` uses, so the two
//! surfaces answer identically). `GET|POST /api/v1/memories/search` and
//! `GET|POST /api/v1/context` live
//! in [`search`]; the mutating routes (`POST /memories`, `DELETE
//! /memories/{id}`, `POST /feedback`) live in [`write`] — both merged into
//! this resource's [`router`], `write`'s own route-table entries appended
//! at [`super::table`] alongside this module's (the `maint`/`prune`
//! sibling-module pattern).

use std::time::Instant;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::get;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond, run_blocking};
use crate::serve::scope::RepoScope;

/// `PATCH /memories/{id}`, `POST /memories/{id}/restore`,
/// `POST /memories/{id}/references/refresh` (`api::{update,restore,refresh_refs}`).
pub mod edit;
/// `GET|POST /memories/search` (`api::search`) and `GET|POST /context`
/// (`api::context`).
pub mod search;
/// `POST /memories`, `DELETE /memories/{id}`, `POST /feedback` — the
/// mutating routes (`api::{save,delete,feedback}`).
pub mod write;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/memories",
            command: "list",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/memories/{id}",
            // `show` — the route runs `api::show::run`, so the table names
            // that command rather than a synthetic one. The parity walk
            // requires every non-cli-only subcommand to own a route (AC-12).
            command: "show",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/memories/search",
            command: "search",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/memories/search",
            command: "search",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/context",
            command: "context",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/context",
            command: "context",
            mutating: false,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/memories", get(list))
        .route("/api/v1/memories/{id}", get(get_one))
        .merge(search::router(state.clone()))
        .merge(write::router(state.clone()))
        .merge(edit::router(state))
}

/// `GET /api/v1/memories` — page live memories (`api::list`). An
/// `X-Comemory-Repo` header is the default `repo` filter when the query
/// omits one ([`RepoScope`]).
async fn list(
    State(state): State<AppState>,
    scope: RepoScope,
    Query(mut req): Query<api::list::Request>,
) -> Response {
    scope.apply(&mut req.repo);
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::list::run(&mut ctx, req)
    })
    .await;
    respond("list", result, started)
}

/// `GET /api/v1/memories/{id}` — single-row lookup (`api::show`), so the
/// HTTP surface and `comemory show --json` answer identically. `404
/// not_found` when the id is absent or soft-deleted.
async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::show::run(&mut ctx, api::show::Request { id })
    })
    .await;
    respond("show", result, started)
}
