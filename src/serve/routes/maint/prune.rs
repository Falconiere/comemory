//! `GET /api/v1/prune` — the dry-run scan report (`api::prune`). The
//! confirm-gated mutating `POST /api/v1/prune` route arrives in a later
//! step; this GET is `mutating: false` and must never soft-delete
//! anything.

use std::time::Instant;

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond, run_blocking};

/// This resource's route-table entries, appended onto [`super::super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[RouteEntry {
        method: "GET",
        path: "/prune",
        command: "prune",
        mutating: false,
    }]
}

/// This resource's routes, merged into the `maint` resource router.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/prune", get(prune))
}

/// `GET /api/v1/prune` — the dry-run report (`api::prune`). `apply` is
/// forced `false` regardless of the query string: a `GET` route carries
/// no confirm gate, so it must never trigger the soft-delete/cleanup path
/// — that lands behind a confirm-gated `POST /api/v1/prune` later.
async fn prune(
    State(state): State<AppState>,
    Query(mut req): Query<api::prune::Request>,
) -> Response {
    req.apply = false;
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::prune::run(&mut ctx, req)
    })
    .await;
    respond("prune", result, started)
}
