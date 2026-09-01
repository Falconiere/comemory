//! `GET /api/v1/stats` (`api::stats`) — corpus counters and database size.
//!
//! Its own resource file rather than an entry under [`super::meta`]: `meta`
//! describes the *surface* (`completions`, `commands`), while this reports
//! the *corpus*. Filing it there would put two unrelated concerns behind one
//! `table_entries()`.

use std::time::Instant;

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond, run_blocking};

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[RouteEntry {
        method: "GET",
        path: "/stats",
        command: "stats",
        mutating: false,
    }]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/stats", get(stats))
}

/// `GET /api/v1/stats` — corpus counters (`api::stats`). Uses `Ctx::lazy`
/// rather than the shared connection so the must-not-create-the-db
/// invariant holds here exactly as it does on the CLI: a server pointed at
/// an empty data dir answers with zeros instead of materializing a
/// database.
async fn stats(
    State(state): State<AppState>,
    scope: crate::serve::scope::RepoScope,
    Query(mut req): Query<api::stats::Request>,
) -> Response {
    scope.fill_if_absent(&mut req.repo);
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::stats::run(&mut ctx, req)
    })
    .await;
    respond("stats", result, started)
}
