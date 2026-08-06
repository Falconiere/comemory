//! `GET /api/v1/doctor` (`api::doctor`) and `GET /api/v1/consolidate`
//! (`api::consolidate`). `GET /api/v1/prune` (`api::prune`, report path)
//! lives in [`prune`], merged into this resource's [`router`]. The
//! mutating maintenance routes (`POST /prune|/gc|/mine`, `POST
//! /hooks/install`) join `prune`/a new `admin` sibling in a later step.

use std::time::Instant;

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond, run_blocking};

/// `GET /api/v1/prune` (`api::prune`, report path).
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
        .merge(prune::router(state))
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
/// (`api::consolidate`).
async fn consolidate(
    State(state): State<AppState>,
    Query(req): Query<api::consolidate::Request>,
) -> Response {
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
