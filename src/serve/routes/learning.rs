//! `POST /api/v1/eval` (`domains::learning::eval`, **job**, read class — no confirm gate,
//! not read-only-gated, AC-4), `GET /api/v1/eval/history` (`domains::learning::eval`'s
//! `history`, synchronous — a `SELECT`, not worth a job), `POST
//! /api/v1/tune` (`domains::learning::tune`, **job**, mutating, confirm only when
//! `req.apply`), and `POST /api/v1/bandit` (`domains::learning::bandit`, **job**, always
//! mutating — it upserts `bandit_arms` regardless of `apply` — confirm
//! only when `req.apply`). `POST /api/v1/feedback` stays in
//! `memories/write.rs`.
//!
//! `eval`/`tune`/`bandit` each contain an optional `golden` file to an
//! allowed root BEFORE any other check (AC-7). A successful `tune`/`bandit`
//! apply job reloads `AppState.cfg` from the rewritten `config.toml` so
//! HTTP ranking picks up the new knobs without a restart (§Route map
//! Notes); a reload failure becomes the job's own error.

use std::path::Path;
use std::time::Instant;

use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;

use crate::domains::learning;
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::envelope::Envelope;
use crate::serve::jobs;
use crate::serve::routes::maint::prune::split_confirm;
use crate::serve::routes::mutate_response::with_lazy_ctx;
use crate::serve::routes::{RouteEntry, accepted, guard_job, require_confirm, respond};
use crate::utilities::blocking::run_blocking;
use crate::utilities::context::Ctx;
use crate::utilities::path_containment;

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
            method: "GET",
            path: "/eval/history",
            command: "eval.history",
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
        .route("/api/v1/eval/history", get(eval_history))
        .route("/api/v1/tune", post(tune))
        .route("/api/v1/bandit", post(bandit))
}

/// Canonicalize `golden` (when present) and require it inside one of this
/// session's allowed roots (§Security "Path containment"), returning the
/// canonicalized path so the caller can overwrite the request field with it
/// rather than re-resolving the raw input downstream. `pub(crate)` because
/// `GET /learning/golden-set` (`routes::learning_console`) contains its own
/// `?golden=` through this exact check rather than a second copy of it. `None` is a no-op —
/// nothing to contain when the run is feedback-harvest-only. Nonexistent
/// path -> `400`; outside every root -> `403`.
pub(crate) fn contain_golden(state: &AppState, golden: Option<&str>) -> Result<Option<String>> {
    let Some(golden) = golden else {
        return Ok(None);
    };
    let conn = state.conn()?;
    let roots = state.allowed_roots(&conn);
    drop(conn);
    let canonical = path_containment::contain_abs(&roots, Path::new(golden))?;
    Ok(Some(canonical.to_string_lossy().into_owned()))
}

/// `POST /api/v1/eval` — start an `eval` job (`domains::learning::eval`). Read class
/// through and through (§Route map Notes): no confirm gate, and — per
/// AC-4 — no read-only gate either, so it stays functional on a
/// `--read-only` server. `req.golden`, when present, is contained to an
/// allowed root before the job is created (AC-7's golden-file half), and a
/// `knobs` override is validated against the live config next — a `400`
/// here, before any job exists, rather than a job that ends `error`
/// (`learning::eval::run` re-checks; this is the synchronous answer).
///
/// `pub(crate)` because `POST /api/v1/learning/evals` (console-api spec §7)
/// is an ALIAS onto this same handler, not a second implementation.
pub(crate) async fn eval(
    State(state): State<AppState>,
    Json(mut req): Json<learning::eval::Request>,
) -> Response {
    let started = Instant::now();
    let contain_state = state.clone();
    let golden = req.golden.clone();
    let contained = run_blocking(move || contain_golden(&contain_state, golden.as_deref())).await;
    match contained {
        Ok(canonical) => req.golden = canonical,
        Err(e) => return Envelope::err("eval", &e, 0),
    }
    if let Err(e) = learning::eval::effective_config(&state.cfg(), req.knobs.as_ref()) {
        return Envelope::err("eval", &e, 0);
    }
    let job_state = state.clone();
    let job = jobs::spawn_job(
        state.jobs(),
        state.write_permit().clone(),
        "eval",
        false,
        move || {
            let resp = with_lazy_ctx(&job_state, |ctx| learning::eval::run(ctx, req))?;
            serde_json::to_value(resp).map_err(Error::Json)
        },
    );
    accepted("eval", job, started)
}

/// `GET /api/v1/eval/history` — read back up to `?limit=` past `eval`/
/// `tune`/`bandit` runs, newest-first (`learning::eval::history`). Query-decoded
/// as the same [`learning::eval::Request`] `POST /eval` uses (AC-41 parity probes
/// one `Request` type per subcommand); `history` only ever reads `limit`
/// from it. A plain `SELECT` against `eval_runs`, so it runs synchronously
/// (`run_blocking`) rather than as a job, matching `GET /memories`'s shape.
async fn eval_history(
    State(state): State<AppState>,
    Query(req): Query<learning::eval::Request>,
) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        learning::eval::history(&mut ctx, &req)
    })
    .await;
    respond("eval.history", result, started)
}

/// What [`spawn_learning_job`] needs from a learning request: the containable
/// `golden` path plus the `apply` flag. Each request type keeps its other
/// fields to itself.
trait LearningJobRequest: serde::de::DeserializeOwned {
    /// Containable golden path, when the run uses one.
    fn golden(&self) -> Option<&str>;
    /// Store the contained golden path back onto the request.
    fn set_golden(&mut self, golden: Option<String>);
    /// Whether the run may rewrite `config.toml`.
    fn apply(&self) -> bool;
}

/// What [`spawn_learning_job`] needs from a learning response: serializable,
/// plus whether `config.toml` was rewritten (triggers `reload_cfg`).
trait LearningJobResponse: serde::Serialize {
    /// Whether `config.toml` was rewritten.
    fn applied(&self) -> bool;
}

/// Generate a `tune`/`bandit`-shaped route: the [`LearningJobRequest`] and
/// [`LearningJobResponse`] impls plus the thin handler delegating to
/// [`spawn_learning_job`].
///
/// One macro, two invocations, for the same reason `wire_strings!` in
/// `domains::architecture::model` exists: writing the impls and handlers by
/// hand gives `scripts/dup-check.sh` near-duplicate bodies to count, and a
/// generic helper alone cannot absorb the per-type impls.
macro_rules! learning_job_route {
    ($handler:ident, $command:literal, $request:ty, $run:path, $response:ty) => {
        impl LearningJobRequest for $request {
            fn golden(&self) -> Option<&str> {
                self.golden.as_deref()
            }
            fn set_golden(&mut self, golden: Option<String>) {
                self.golden = golden;
            }
            fn apply(&self) -> bool {
                self.apply
            }
        }

        impl LearningJobResponse for $response {
            fn applied(&self) -> bool {
                self.applied
            }
        }

        /// Route handler; the shared shape lives on [`spawn_learning_job`].
        async fn $handler(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
            spawn_learning_job(&state, $command, body, $run).await
        }
    };
}

learning_job_route!(
    tune,
    "tune",
    learning::tune::Request,
    learning::tune::run,
    learning::tune::Response
);
learning_job_route!(
    bandit,
    "bandit",
    learning::bandit::Request,
    learning::bandit::run,
    learning::evaluation::bandit::BanditReport
);

/// Start a `tune`/`bandit`-shaped learning job. The body is a raw [`Value`]
/// read through [`split_confirm`] so the HTTP-only `confirm` flag never joins
/// the domain `Request` (AC-12 parity). Gate order: `golden` containment
/// first (before any other check, including read-only — AC-7), then the
/// read-only gate (`405`, these routes ARE mutating regardless of `apply`),
/// then — only when `req.apply` — the confirm gate (AC-19: read-only still
/// outranks confirm). A successful apply job reloads `AppState.cfg` so HTTP
/// ranking picks up the new knobs without a restart.
async fn spawn_learning_job<Req, Resp>(
    state: &AppState,
    command: &'static str,
    body: Value,
    run: impl FnOnce(&mut Ctx<'_>, Req) -> Result<Resp> + Send + 'static,
) -> Response
where
    Req: LearningJobRequest + Send + 'static,
    Resp: LearningJobResponse,
{
    let started = Instant::now();
    let (mut req, confirmed) = match split_confirm::<Req>(body) {
        Ok(v) => v,
        Err(e) => return Envelope::err(command, &e, 0),
    };
    let contain_state = state.clone();
    let golden = req.golden().map(str::to_owned);
    let contained = run_blocking(move || contain_golden(&contain_state, golden.as_deref())).await;
    match contained {
        Ok(canonical) => req.set_golden(canonical),
        Err(e) => return Envelope::err(command, &e, 0),
    }
    if let Err(resp) = guard_job(command, state) {
        return *resp;
    }
    if req.apply()
        && let Err(e) = require_confirm(confirmed)
    {
        return Envelope::err(command, &e, 0);
    }
    let job_state = state.clone();
    let job = jobs::spawn_job(
        state.jobs(),
        state.write_permit().clone(),
        command,
        true,
        move || {
            let resp = with_lazy_ctx(&job_state, |ctx| run(ctx, req))?;
            if resp.applied() {
                job_state.reload_cfg(job_state.paths())?;
            }
            serde_json::to_value(resp).map_err(Error::Json)
        },
    );
    accepted(command, job, started)
}

#[cfg(test)]
#[path = "tests/learning.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/learning_2.rs"]
mod tests_2;
