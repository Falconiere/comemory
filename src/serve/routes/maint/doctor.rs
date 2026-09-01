//! `GET /api/v1/doctor/system`, `POST /api/v1/doctor/rebuild`,
//! `POST /api/v1/doctor/reembed` (console-api spec §8).
//!
//! `POST /doctor/rebuild` is an **alias**, not a second implementation: it
//! normalizes the draft's `{scope, repo}` envelope away and then calls
//! [`super::admin::rebuild`] — the same handler `POST /api/v1/rebuild`
//! mounts, with the same confirm gate, the same job, and the same
//! post-rebuild connection swap. Only `scope: "all"` exists (a per-repo
//! rebuild would have to re-derive memories from a subset of markdown,
//! which is not a thing `api::rebuild` can do), so anything else is a
//! `400` before the job is created.

use std::sync::Arc;
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
use crate::serve::jobs::{self, worker::RegistryProgressSink};
use crate::serve::routes::{RouteEntry, accepted, guard_job, respond, run_blocking};

/// What `POST /doctor/reembed` answers with when the server was started
/// without an embed command — the one thing this route cannot do without.
const NO_EMBED_CMD: &str =
    "no embed command configured; start serve with --embed-cmd or COMEMORY_EMBED_CMD";

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/doctor/system",
            command: "doctor.system",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/doctor/rebuild",
            command: "rebuild",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/doctor/reembed",
            command: "doctor.reembed",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/doctor/system", get(system))
        .route("/api/v1/doctor/rebuild", post(rebuild))
        .route("/api/v1/doctor/reembed", post(reembed))
}

/// `GET /api/v1/doctor/system` — the probe-free facts read
/// (`api::doctor::system`). Uses `Ctx::lazy`, and the core itself opens the
/// DB only when it already exists, so polling this endpoint on a fresh data
/// dir never creates one. `embed_cmd` is this server's own configured
/// command ([`AppState::embed_cmd`]) — the one `/health` and
/// `POST /doctor/reembed` act on — so the two can never disagree.
async fn system(State(state): State<AppState>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::doctor::system::run(&mut ctx, state.embed_cmd())
    })
    .await;
    respond("doctor.system", result, started)
}

/// `POST /api/v1/doctor/rebuild` — the draft's spelling of `POST
/// /api/v1/rebuild`. Runs the read-only gate FIRST (AC-19: read-only
/// outranks every other refusal, so a `--read-only` server answers `405`
/// even to a body whose `scope` would otherwise be a `400`), then strips
/// the two draft-only keys (see [`strip_scope`]) and delegates; the confirm
/// gate and the job itself live in [`super::admin::rebuild`], which
/// re-checks read-only harmlessly.
async fn rebuild(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    if let Err(resp) = guard_job("rebuild", &state) {
        return *resp;
    }
    let body = match strip_scope(body) {
        Ok(body) => body,
        Err(e) => return Envelope::err("rebuild", &e, 0),
    };
    super::admin::rebuild(State(state), Json(body)).await
}

/// Remove the draft's `scope` / `repo` keys from a raw `POST
/// /doctor/rebuild` body, rejecting any value that would mean something
/// this endpoint cannot do.
///
/// They must be *removed*, not merely inspected: the shared handler reads
/// the remainder as `api::rebuild::Request`, which is
/// `deny_unknown_fields`, so leaving them in would turn a spec-conformant
/// body into a deserialization error. A non-object body is passed through
/// untouched — the shared handler's own `split_confirm` already answers it
/// with the right error.
fn strip_scope(mut body: Value) -> Result<Value> {
    let Some(obj) = body.as_object_mut() else {
        return Ok(body);
    };
    match obj.remove("scope") {
        None | Some(Value::Null) => {}
        Some(Value::String(scope)) if scope == "all" => {}
        Some(other) => {
            return Err(Error::BadRequest(format!(
                "per-repo rebuild is not supported: scope must be \"all\", got {other}"
            )));
        }
    }
    match obj.remove("repo") {
        None | Some(Value::Null) => Ok(body),
        Some(_) => Err(Error::BadRequest(
            "per-repo rebuild is not supported: repo must be null".into(),
        )),
    }
}

/// `POST /api/v1/doctor/reembed` — start a `reembed` job
/// (`api::reembed::run`) over the server's own embed command.
///
/// Gate order: read-only first ([`guard_job`] → `405 read_only`), then the
/// embed-command check → `503 embedder_unavailable` **before** a job is
/// registered, so a misconfigured server never leaves a job that could only
/// ever fail. Not confirm-gated: re-embedding replaces vectors that are
/// derived data by definition, and the rows themselves are untouched.
///
/// The job runs under a [`RegistryProgressSink`] built from its own id, so
/// per-row progress and log lines land in `JobView` and stream out over
/// `GET /jobs/{id}/events`, and `POST /jobs/{id}/cancel` stops the run at
/// the next row boundary.
async fn reembed(
    State(state): State<AppState>,
    Json(req): Json<api::reembed::Request>,
) -> Response {
    let started = Instant::now();
    if let Err(resp) = guard_job("doctor.reembed", &state) {
        return *resp;
    }
    let Some(cmd) = state.embed_cmd().map(str::to_string) else {
        return Envelope::err(
            "doctor.reembed",
            &Error::Embedder(NO_EMBED_CMD.to_string()),
            0,
        );
    };
    let registry = Arc::clone(state.jobs());
    let job_state = state.clone();
    let job = jobs::spawn_job_with_id(
        state.jobs(),
        state.write_permit().clone(),
        "reembed",
        true,
        move |job_id| {
            let sink = RegistryProgressSink::new(registry, job_id);
            let cfg = job_state.cfg();
            let mut ctx = Ctx::lazy(job_state.paths(), &cfg);
            let resp = api::reembed::run(&mut ctx, req, &cmd, Some(&sink))?;
            serde_json::to_value(resp).map_err(Error::Json)
        },
    );
    accepted("doctor.reembed", job, started)
}

#[cfg(test)]
#[path = "../tests/maint_doctor.rs"]
mod tests;
