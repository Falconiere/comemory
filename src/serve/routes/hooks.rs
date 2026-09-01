//! `GET /api/v1/hooks` (read hook state, `api::hooks`) and `POST
//! /api/v1/hooks` (per-hook enable/disable, `api::hooks`) — the console's
//! readable, per-hook-controllable surface over `install-hooks`'s three git
//! hooks plus the config-backed search→edit auto-reinforcement row.
//!
//! Not confirm-gated (spec AC-33b): installing or removing a hook file is
//! idempotent and reversible, unlike `POST /api/v1/hooks/install`
//! (`maint::admin`), which stays confirm-gated as the install-all
//! shorthand. `POST` still goes through [`guard_mutating`] — `405
//! read_only` on a `--read-only` server, `503 busy` on write-permit
//! contention.

use std::time::Instant;

use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, guard_mutating, respond, run_blocking};

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/hooks",
            command: "hooks",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/hooks",
            command: "hooks",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/hooks", get(list_hooks).post(toggle_hooks))
}

/// The only query parameter `GET /api/v1/hooks` reads. Deliberately narrower
/// than `api::hooks::Request` (which also carries `enable`/`disable`) — a
/// `GET` must never be able to toggle a hook no matter what a client puts on
/// the query string, so this handler builds the request itself rather than
/// deserializing the full type straight off the query.
#[derive(Deserialize, Debug)]
struct ListQuery {
    #[serde(default)]
    repo: Option<String>,
}

/// `GET /api/v1/hooks` — report all four rows (`api::hooks`), read-only.
async fn list_hooks(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::hooks::run(
            &mut ctx,
            api::hooks::Request {
                repo: q.repo,
                enable: None,
                disable: None,
            },
        )
    })
    .await;
    respond("hooks", result, started)
}

/// `POST /api/v1/hooks` — enable/disable one hook (`api::hooks`), then
/// report all four rows. Read-only-gated via [`guard_mutating`]; not
/// confirm-gated (module doc, AC-33b).
async fn toggle_hooks(
    State(state): State<AppState>,
    Json(req): Json<api::hooks::Request>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("hooks", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::hooks::run(&mut ctx, req)
    })
    .await;
    respond("hooks", result, started)
}
