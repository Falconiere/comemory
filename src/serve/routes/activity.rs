//! `GET /api/v1/activity` (`maintenance::activity`) — the filtered,
//! newest-first feed of recorded command runs, with its per-command rollups.
//!
//! Its own resource file rather than an entry under [`super::stats`] or
//! [`super::overview`]: those two report the *corpus*, this reports the
//! *traffic*. A synthetic console name with no CLI counterpart, like
//! `overview` and `learning.summary`.
//!
//! Uses [`Ctx::lazy`] rather than the shared connection for the reason
//! [`super::stats`] does: a server pointed at an empty data dir must answer
//! with an empty feed instead of materializing a database.

use std::time::Instant;

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;

use crate::domains::maintenance;
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond};
use crate::serve::scope::RepoScope;
use crate::utilities::blocking::run_blocking;
use crate::utilities::context::Ctx;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[RouteEntry {
        method: "GET",
        path: "/activity",
        command: "activity",
        mutating: false,
    }]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/activity", get(activity))
}

/// `GET /api/v1/activity` — one page of recorded runs plus its rollups.
async fn activity(
    State(state): State<AppState>,
    scope: RepoScope,
    Query(mut req): Query<maintenance::activity::Request>,
) -> Response {
    req.repo = scope.resolve(req.repo);
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        maintenance::activity::run(&mut ctx, req)
    })
    .await;
    respond("activity", result, started)
}
