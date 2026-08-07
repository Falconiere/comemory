//! `POST /api/v1/eval` (`api::eval`, **job**, read class — no confirm gate,
//! not read-only-gated, AC-4), `POST /api/v1/tune` (`api::tune`, **job**,
//! mutating, confirm only when `req.apply`), and `POST /api/v1/bandit`
//! (`api::bandit`, **job**, always mutating — it upserts `bandit_arms`
//! regardless of `apply` — confirm only when `req.apply`). `POST
//! /api/v1/feedback` stays in `memories/write.rs`.
//!
//! All three contain an optional `golden` file to an allowed root BEFORE
//! any other check (AC-7). A successful `tune`/`bandit` apply job reloads
//! `AppState.cfg` from the rewritten `config.toml` so HTTP ranking picks up
//! the new knobs without a restart (§Route map Notes); a reload failure
//! becomes the job's own error.

use std::path::Path;
use std::time::Instant;

use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::Value;

use crate::api::{self, Ctx};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::envelope::Envelope;
use crate::serve::jobs;
use crate::serve::routes::maint::prune::split_confirm;
use crate::serve::routes::{RouteEntry, accepted, guard_job, require_confirm, run_blocking};
use crate::serve::security;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "POST",
            path: "/eval",
            command: "eval",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/tune",
            command: "tune",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/bandit",
            command: "bandit",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/eval", post(eval))
        .route("/api/v1/tune", post(tune))
        .route("/api/v1/bandit", post(bandit))
}

/// Canonicalize `golden` (when present) and require it inside one of this
/// session's allowed roots (§Security "Path containment"). `None` is a
/// no-op — nothing to contain when the run is feedback-harvest-only.
/// Nonexistent path -> `400`; outside every root -> `403`.
fn contain_golden(state: &AppState, golden: Option<&str>) -> Result<()> {
    let Some(golden) = golden else {
        return Ok(());
    };
    let conn = state.conn()?;
    let roots = state.allowed_roots(&conn);
    drop(conn);
    security::contain_abs(&roots, Path::new(golden))?;
    Ok(())
}

/// `POST /api/v1/eval` — start an `eval` job (`api::eval`). Read class
/// through and through (§Route map Notes): no confirm gate, and — per
/// AC-4 — no read-only gate either, so it stays functional on a
/// `--read-only` server. `req.golden`, when present, is contained to an
/// allowed root before the job is created (AC-7's golden-file half).
async fn eval(State(state): State<AppState>, Json(req): Json<api::eval::Request>) -> Response {
    let started = Instant::now();
    let contain_state = state.clone();
    let golden = req.golden.clone();
    let contained = run_blocking(move || contain_golden(&contain_state, golden.as_deref())).await;
    if let Err(e) = contained {
        return Envelope::err("eval", &e, 0);
    }
    let job_state = state.clone();
    let job = jobs::spawn_job(
        state.jobs(),
        state.write_permit().clone(),
        "eval",
        false,
        move || {
            let cfg = job_state.cfg();
            let mut ctx = Ctx::lazy(job_state.paths(), &cfg);
            let resp = api::eval::run(&mut ctx, req)?;
            serde_json::to_value(resp).map_err(Error::Json)
        },
    );
    accepted("eval", job, started)
}

/// `POST /api/v1/tune` — start a `tune` job (`api::tune`). The body is a
/// raw [`Value`] read through [`split_confirm`] so the HTTP-only `confirm`
/// flag never joins `api::tune::Request` (AC-12 parity). Gate order:
/// `golden` containment first (before any other check, including
/// read-only — AC-7), then the read-only gate (`405`, this route IS
/// mutating regardless of `apply`), then — only when `req.apply` — the
/// confirm gate (AC-19: read-only still outranks confirm).
async fn tune(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let started = Instant::now();
    let (req, confirmed) = match split_confirm::<api::tune::Request>(body) {
        Ok(v) => v,
        Err(e) => return Envelope::err("tune", &e, 0),
    };
    let contain_state = state.clone();
    let golden = req.golden.clone();
    let contained = run_blocking(move || contain_golden(&contain_state, golden.as_deref())).await;
    if let Err(e) = contained {
        return Envelope::err("tune", &e, 0);
    }
    if let Err(resp) = guard_job("tune", &state) {
        return *resp;
    }
    if req.apply
        && let Err(e) = require_confirm(confirmed)
    {
        return Envelope::err("tune", &e, 0);
    }
    let job_state = state.clone();
    let job = jobs::spawn_job(
        state.jobs(),
        state.write_permit().clone(),
        "tune",
        true,
        move || {
            let cfg = job_state.cfg();
            let mut ctx = Ctx::lazy(job_state.paths(), &cfg);
            let resp = api::tune::run(&mut ctx, req)?;
            if resp.applied {
                job_state.reload_cfg(job_state.paths())?;
            }
            serde_json::to_value(resp).map_err(Error::Json)
        },
    );
    accepted("tune", job, started)
}

/// `POST /api/v1/bandit` — start a `bandit` job (`api::bandit`). Same
/// body/confirm/gate shape as [`tune`]: `golden` containment, then the
/// read-only gate (this route is always mutating — `api::bandit::run`
/// upserts `bandit_arms` regardless of `apply`), then confirm only when
/// `req.apply`.
async fn bandit(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let started = Instant::now();
    let (req, confirmed) = match split_confirm::<api::bandit::Request>(body) {
        Ok(v) => v,
        Err(e) => return Envelope::err("bandit", &e, 0),
    };
    let contain_state = state.clone();
    let golden = req.golden.clone();
    let contained = run_blocking(move || contain_golden(&contain_state, golden.as_deref())).await;
    if let Err(e) = contained {
        return Envelope::err("bandit", &e, 0);
    }
    if let Err(resp) = guard_job("bandit", &state) {
        return *resp;
    }
    if req.apply
        && let Err(e) = require_confirm(confirmed)
    {
        return Envelope::err("bandit", &e, 0);
    }
    let job_state = state.clone();
    let job = jobs::spawn_job(
        state.jobs(),
        state.write_permit().clone(),
        "bandit",
        true,
        move || {
            let cfg = job_state.cfg();
            let mut ctx = Ctx::lazy(job_state.paths(), &cfg);
            let resp = api::bandit::run(&mut ctx, req)?;
            if resp.applied {
                job_state.reload_cfg(job_state.paths())?;
            }
            serde_json::to_value(resp).map_err(Error::Json)
        },
    );
    accepted("bandit", job, started)
}
