//! `GET|PUT /api/v1/gc/policy`, `POST /api/v1/gc/run` (console-api spec §9).
//!
//! The two retention windows `comemory gc` reads live in `config.toml`, so
//! `PUT /gc/policy` is a config writer (`api::gc_policy::update`) followed
//! by an `AppState::reload_cfg` — without the reload, `POST /gc/run` on the
//! same server would keep sweeping under the pre-`PUT` windows until
//! restart, which is exactly the surprise AC-17 exists to rule out.
//!
//! The synchronous `POST /api/v1/gc` in [`super::prune`] stays: it is the
//! shape a script wants (one call, one answer). `POST /gc/run` is the
//! job-backed form for a console that needs a progress-pollable id on a
//! trash directory large enough for the sweep to take a while.

use std::time::Instant;

use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;

use crate::api::{self, Ctx};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::envelope::Envelope;
use crate::serve::jobs;
use crate::serve::routes::maint::prune::split_confirm;
use crate::serve::routes::{
    RouteEntry, accepted, guard_job, guard_mutating, require_confirm, respond, run_blocking,
};

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/gc/policy",
            command: "gc.policy",
            mutating: false,
        },
        RouteEntry {
            method: "PUT",
            path: "/gc/policy",
            command: "gc.policy.update",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/gc/run",
            command: "gc.run",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/gc/policy", get(policy).put(update_policy))
        .route("/api/v1/gc/run", post(run))
}

/// `GET /api/v1/gc/policy` — the live retention windows plus the newest
/// `gc_runs` row (`api::gc_policy::get`). `Ctx::lazy`: the core reads the
/// DB only when it already exists, so this never creates one.
async fn policy(State(state): State<AppState>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::gc_policy::get(&mut ctx)
    })
    .await;
    respond("gc.policy", result, started)
}

/// `PUT /api/v1/gc/policy` — patch one or both windows into `config.toml`
/// (`api::gc_policy::update`), then reload the server's shared config so
/// the new windows take effect immediately (module doc).
///
/// Not confirm-gated: it writes two integers into `config.toml`, reversibly
/// and idempotently — the same reasoning that leaves `POST /hooks`
/// unconfirmed. An out-of-range value is `400 bad_request` from the core's
/// validate-before-write, with the file untouched.
async fn update_policy(
    State(state): State<AppState>,
    Json(req): Json<api::gc_policy::UpdateRequest>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("gc.policy.update", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        let policy = api::gc_policy::update(&mut ctx, req)?;
        // Only after the file is written: a failed write must not swap in a
        // config the file does not back.
        state.reload_cfg(state.paths())?;
        Ok(policy)
    })
    .await;
    respond("gc.policy.update", result, started)
}

/// `POST /api/v1/gc/run` — the job-backed trash + telemetry sweep
/// (`api::gc`). Gate order (AC-19): read-only first ([`guard_job`] →
/// `405 read_only`), then the confirm gate — `gc` hard-deletes trashed
/// markdown, which is the one prune-family step with nothing behind it.
async fn run(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let started = Instant::now();
    if let Err(resp) = guard_job("gc.run", &state) {
        return *resp;
    }
    if let Err(e) = split_confirm::<api::gc::Request>(body)
        .and_then(|(_req, confirmed)| require_confirm(confirmed))
    {
        return Envelope::err("gc.run", &e, 0);
    }
    let job_state = state.clone();
    let job = jobs::spawn_job(
        state.jobs(),
        state.write_permit().clone(),
        "gc",
        true,
        move || {
            let cfg = job_state.cfg();
            let mut ctx = Ctx::lazy(job_state.paths(), &cfg);
            let resp = api::gc::run(&mut ctx, api::gc::Request {})?;
            serde_json::to_value(resp).map_err(Error::Json)
        },
    );
    accepted("gc.run", job, started)
}

#[cfg(test)]
#[path = "../tests/maint_gc.rs"]
mod tests;
