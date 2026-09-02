//! `GET /api/v1/overview`, `GET /api/v1/overview/eval-series` (console-api spec §2).
//!
//! Both are pure reads with no CLI counterpart, so their `RouteEntry`
//! commands are the dotted synthetic names `overview` / `overview.eval-series`.
//!
//! Both use [`Ctx::lazy`] rather than the shared connection, for the same
//! reason [`super::stats`] does: `api::overview` keeps the
//! must-not-create-the-db invariant, and borrowing the already-open shared
//! connection would defeat it — a server pointed at an empty data dir would
//! materialize a database just by having its Overview screen opened.

use std::time::Instant;

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use serde::Deserialize;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond, run_blocking};
use crate::serve::scope::RepoScope;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/overview",
            command: "overview",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/overview/eval-series",
            command: "overview.eval-series",
            mutating: false,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/overview", get(overview))
        .route("/api/v1/overview/eval-series", get(eval_series))
}

/// `GET /api/v1/overview` — the landing aggregate. An `X-Comemory-Repo`
/// header (or the server's `--repo`) is the default scope when the query
/// omits `repo`.
async fn overview(
    State(state): State<AppState>,
    scope: RepoScope,
    Query(mut req): Query<api::overview::Request>,
) -> Response {
    req.repo = scope.resolve(req.repo);
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::overview::run(&mut ctx, req)
    })
    .await;
    respond("overview", result, started)
}

/// `?limit=` on `GET /overview/eval-series` — transport-level, since
/// `api::overview::eval_series` takes a plain `u32` rather than a `Request`
/// struct. No repo scope: `eval_runs` is a global history (an eval run
/// scores the whole golden set, not one repo's slice of it).
#[derive(Deserialize)]
struct EvalSeriesQuery {
    /// How many runs to plot, newest-selected but oldest-first in the
    /// answer. Defaults to [`api::overview::EVAL_SERIES_LIMIT`].
    #[serde(default)]
    limit: Option<u32>,
}

/// `GET /api/v1/overview/eval-series` — the recall sparkline on its own.
async fn eval_series(
    State(state): State<AppState>,
    Query(query): Query<EvalSeriesQuery>,
) -> Response {
    let started = Instant::now();
    let limit = query.limit.unwrap_or(api::overview::EVAL_SERIES_LIMIT);
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::overview::eval_series(&mut ctx, limit)
    })
    .await;
    respond("overview.eval-series", result, started)
}

#[cfg(test)]
#[path = "tests/overview.rs"]
mod tests;
