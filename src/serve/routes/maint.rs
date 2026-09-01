//! `GET /api/v1/doctor` (`api::doctor`) and `GET /api/v1/consolidate`
//! (`api::consolidate`). `GET|POST /api/v1/prune` and `POST /api/v1/gc`
//! live in [`prune`]; `POST /api/v1/mine` and `POST /api/v1/hooks/install`
//! live in [`admin`] — both merged into this resource's [`router`].

use std::time::Instant;

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond, run_blocking};
use crate::serve::scope::RepoScope;

/// `POST /api/v1/mine`, `POST /api/v1/hooks/install`.
pub mod admin;
/// `GET /api/v1/doctor/system`, `POST /api/v1/doctor/rebuild`, `POST /api/v1/doctor/reembed`.
pub mod doctor;
/// `GET|PUT /api/v1/gc/policy`, `POST /api/v1/gc/run`.
pub mod gc;
/// `GET|POST /api/v1/prune`, `POST /api/v1/gc`.
pub mod prune;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/doctor",
            command: "doctor",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/consolidate",
            command: "consolidate",
            mutating: false,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/doctor", get(doctor))
        .route("/api/v1/consolidate", get(consolidate))
        .merge(prune::router(state.clone()))
        .merge(admin::router(state.clone()))
        .merge(doctor::router(state.clone()))
        .merge(gc::router(state))
}

/// `GET /api/v1/doctor` — data-dir + DB health probe (`api::doctor`). Uses
/// `Ctx::lazy` (never the shared connection) — see `api::doctor`'s doc for
/// why the DB must not be opened eagerly.
async fn doctor(State(state): State<AppState>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::doctor::run(&mut ctx, api::doctor::Request {})
    })
    .await;
    respond("doctor", result, started)
}

/// `GET /api/v1/consolidate` — advisory near-duplicate cluster report
/// (`api::consolidate`). The `repo` filter defaults from the request scope
/// when the query omits one ([`RepoScope`]), like every other repo-bearing
/// read.
async fn consolidate(
    State(state): State<AppState>,
    scope: RepoScope,
    Query(mut req): Query<api::consolidate::Request>,
) -> Response {
    scope.apply(&mut req.repo);
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::consolidate::run(&mut ctx, req)
    })
    .await;
    respond("consolidate", result, started)
}

#[cfg(test)]
#[path = "tests/consolidate.rs"]
mod tests;
