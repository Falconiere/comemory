//! `POST /api/v1/index/runs`, `GET /api/v1/index/runs` (console-api spec §6).
//!
//! `POST` is the console's own index-run entry point: the same `index-code`
//! job `POST /api/v1/code/index` and `POST /api/v1/repos {index_now}`
//! start, with the two gates every one of them applies — an archived repo
//! is refused (`400`, `api::index_code::refuse_if_archived`, which the core
//! run re-checks itself) and a repo that already has a live run is refused
//! (`409 index_running`, AC-10). All three entry points share
//! [`spawn_index_job`] and [`refuse_if_running`], so the job they create and
//! the conflict they report cannot drift.
//!
//! `GET` pages `index_runs` (`api::index_runs`), the history every run —
//! CLI, `POST /code/index`, or `POST /index/runs` — writes one row into.

use std::path::{Path, PathBuf};
use std::time::Instant;

use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::index_code::IndexMode;
use crate::api::{self, Ctx};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::envelope::Envelope;
use crate::serve::jobs::{self, JobId};
use crate::serve::routes::{RouteEntry, accepted, guard_job, respond, run_blocking};
use crate::serve::scope::RepoScope;
use crate::serve::security;

/// The job/registry command name every index run is registered under —
/// the CLI subcommand's own name, shared with `POST /api/v1/code/index` so
/// `Registry::active_for` sees one job kind, not two.
pub(crate) const INDEX_JOB_COMMAND: &str = "index-code";

/// The route-table / envelope command for `POST /index/runs`.
const RUN_COMMAND: &str = "index.run";

/// The route-table / envelope command for `GET /index/runs`.
const RUNS_COMMAND: &str = "index.runs";

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/index/runs",
            command: RUNS_COMMAND,
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/index/runs",
            command: RUN_COMMAND,
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/index/runs", get(list_runs).post(start_run))
}

/// `GET /api/v1/index/runs` — the paged run history (`api::index_runs`).
/// Uses `Ctx::lazy` so the must-not-create-the-db invariant holds: a server
/// pointed at an empty data dir answers with an empty page.
async fn list_runs(
    State(state): State<AppState>,
    scope: RepoScope,
    Query(mut req): Query<api::index_runs::Request>,
) -> Response {
    req.repo = scope.resolve(req.repo);
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::index_runs::run(&mut ctx, req)
    })
    .await;
    respond(RUNS_COMMAND, result, started)
}

/// `POST /api/v1/index/runs` body. `root` is an accepted alias of `path`,
/// and `paths` the console's array form — exactly one root per run, so a
/// multi-element `paths` is a `400` rather than a silently-dropped tail.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct StartRequest {
    /// Repo label to index under.
    repo: String,
    /// Working-tree root to walk.
    #[serde(default)]
    path: Option<String>,
    /// Alias of `path`.
    #[serde(default)]
    root: Option<String>,
    /// `incremental` (default) or `full`.
    #[serde(default)]
    mode: Option<IndexMode>,
    /// Array form of `path`; at most one element.
    #[serde(default)]
    paths: Option<Vec<String>>,
}

/// A validated, contained, conflict-free index run, ready to spawn.
struct IndexPlan {
    repo: String,
    path: String,
    mode: IndexMode,
}

/// `POST /api/v1/index/runs` — start an `index-code` job. Gate order:
/// read-only ([`guard_job`] → `405`), then archived (`400`), then a live
/// run for the same repo (`409 index_running`, AC-10), then containment of
/// the root to an allowed root (`403`/`400`) — every one of them BEFORE a
/// job exists, so a refused request never leaves a job behind.
async fn start_run(State(state): State<AppState>, Json(req): Json<StartRequest>) -> Response {
    let started = Instant::now();
    if let Err(resp) = guard_job(RUN_COMMAND, &state) {
        return *resp;
    }
    let plan_state = state.clone();
    let planned = run_blocking(move || plan_run(&plan_state, req)).await;
    let plan = match planned {
        Ok(plan) => plan,
        Err(e) => return Envelope::err(RUN_COMMAND, &e, 0),
    };
    let job = spawn_index_job(&state, plan.repo, plan.path, plan.mode);
    accepted(RUN_COMMAND, job, started)
}

/// The blocking half of [`start_run`]: resolve the root, run the archived
/// and already-running gates, and contain the root.
fn plan_run(state: &AppState, req: StartRequest) -> Result<IndexPlan> {
    let path = resolve_path(&req)?;
    let conn = state.conn()?;
    api::index_code::refuse_if_archived(&conn, &req.repo)?;
    let roots = state.allowed_roots(&conn);
    drop(conn);
    refuse_if_running(state, &req.repo)?;
    let canonical = contained(&roots, &path)?;
    Ok(IndexPlan {
        repo: req.repo,
        path: canonical,
        mode: req.mode.unwrap_or_default(),
    })
}

/// The one root a [`StartRequest`] names, across its three spellings.
/// Zero or more than one is a `400` — an ambiguous request must not pick a
/// root for the caller.
fn resolve_path(req: &StartRequest) -> Result<String> {
    if let Some(paths) = &req.paths
        && paths.len() > 1
    {
        return Err(Error::BadRequest("one root per run".into()));
    }
    let from_array = req.paths.as_ref().and_then(|p| p.first()).cloned();
    let mut given = [req.path.clone(), req.root.clone(), from_array]
        .into_iter()
        .flatten();
    let (Some(path), None) = (given.next(), given.next()) else {
        return Err(Error::BadRequest(
            "exactly one of `path`, `root`, or a single-element `paths` is required".into(),
        ));
    };
    Ok(path)
}

/// Canonicalize `path` inside an allowed root, as the `String` an
/// `api::index_code::Request` carries.
fn contained(roots: &[PathBuf], path: &str) -> Result<String> {
    let canonical = security::contain_abs(roots, Path::new(path))?;
    Ok(canonical.to_string_lossy().into_owned())
}

/// `Err(Error::IndexRunning)` (→ `409` with `details.job_id`) when `repo`
/// already has a queued or running `index-code` job. Shared with
/// `POST /api/v1/code/index` and `POST /api/v1/repos {index_now}` so the
/// three entry points agree (AC-10).
///
/// Safe from a `spawn_blocking` closure ([`plan_run`], `POST /code/index`)
/// and from an async handler alike: `Registry::active_for` holds the job
/// map's `std::sync::Mutex` for one synchronous scan, nothing in `serve`
/// holds that guard across an `.await`, and callers never hold the
/// shared-connection guard at the same time ([`plan_run`] drops it first).
pub(crate) fn refuse_if_running(state: &AppState, repo: &str) -> Result<()> {
    if let Some(job_id) = state.jobs().active_for(INDEX_JOB_COMMAND, repo)? {
        return Err(Error::IndexRunning {
            repo: repo.to_string(),
            job_id,
        });
    }
    Ok(())
}

/// Spawn the `index-code` job both index entry points and `POST /repos
/// {index_now:true}` create: labelled with `repo` (so
/// [`refuse_if_running`] can find it) and wired to a
/// [`jobs::worker::RegistryProgressSink`] so the walk's per-file progress
/// and log lines stream out of `GET /jobs/{id}/events`.
///
/// `path` must already be contained by the caller — this function performs
/// no containment of its own.
pub(crate) fn spawn_index_job(
    state: &AppState,
    repo: String,
    path: String,
    mode: IndexMode,
) -> Result<JobId> {
    let label = repo.clone();
    let job_state = state.clone();
    jobs::spawn_job_for(
        state.jobs(),
        state.write_permit().clone(),
        INDEX_JOB_COMMAND,
        Some(&label),
        true,
        move |job_id| {
            let cfg = job_state.cfg();
            let mut ctx = Ctx::lazy(job_state.paths(), &cfg);
            let sink = jobs::worker::RegistryProgressSink::new(job_state.jobs().clone(), job_id);
            let resp = api::index_code::run_with_progress(
                &mut ctx,
                api::index_code::Request { repo, path, mode },
                Some(&sink),
            )?;
            serde_json::to_value(resp).map_err(Error::Json)
        },
    )
}

#[cfg(test)]
#[path = "tests/index_runs.rs"]
mod tests;
