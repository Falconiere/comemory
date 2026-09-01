//! `POST /api/v1/mine` (`api::mine`, no confirm gate — a bounded scan that
//! only mutates when `"apply":true`), `POST /api/v1/hooks/install`
//! (`api::install_hooks`, confirm-gated, `--repo` contained), and
//! `POST /api/v1/rebuild` (`api::rebuild`) — a confirm-gated **job** that
//! also swaps the server's shared connection onto the freshly built DB.

use std::path::{Path, PathBuf};
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
use crate::serve::routes::{
    RouteEntry, accepted, guard_job, guard_mutating, require_confirm, respond, run_blocking,
};
use crate::serve::security;

/// This resource's route-table entries, appended onto [`super::table_entries`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "POST",
            path: "/mine",
            command: "mine",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/hooks/install",
            command: "install-hooks",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/rebuild",
            command: "rebuild",
            mutating: true,
        },
    ]
}

/// This resource's routes, merged into the `maint` resource router.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/mine", post(mine))
        .route("/api/v1/hooks/install", post(hooks_install))
        .route("/api/v1/rebuild", post(rebuild))
}

/// `POST /api/v1/mine` — distill (and, with `"apply":true`, rebuild)
/// `query_expansions` (`api::mine`). Not confirm-gated (§Route map): it is a
/// bounded scan, mutating only on explicit `apply`.
async fn mine(State(state): State<AppState>, Json(req): Json<api::mine::Request>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("mine", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::mine::run(&mut ctx, req)
    })
    .await;
    respond("mine", result, started)
}

/// `POST /api/v1/hooks/install` — install git hooks (`api::install_hooks`),
/// confirm-gated with `--repo` contained to an allowed root. The body is
/// read as a raw JSON [`Value`] (via [`split_confirm`], shared with
/// `maint::prune`'s `POST /prune`) so the HTTP-only `confirm` flag never
/// joins `api::install_hooks::Request` itself (AC-12 parity).
async fn hooks_install(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("hooks.install", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let (mut req, confirmed) = split_confirm::<api::install_hooks::Request>(body)?;
        let canonical = contain_repo(&state, &req.repo)?;
        req.repo = canonical.to_string_lossy().into_owned();
        require_confirm(confirmed)?;
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::install_hooks::run(&mut ctx, req)
    })
    .await;
    respond("hooks.install", result, started)
}

/// `POST /api/v1/rebuild` — start a `rebuild` job (`api::rebuild`), then
/// swap the server's shared connection onto the freshly built DB.
///
/// Gate order (AC-19): read-only first ([`guard_job`] → `405 read_only`,
/// never `503 busy` — a job-creating `POST` always answers immediately and
/// waits for the write permit inside the job), then the confirm gate. The
/// body is a raw [`Value`] read through [`split_confirm`] so the HTTP-only
/// `confirm` flag never joins `api::rebuild::Request` (AC-12 parity).
///
/// The job body runs the two steps in sequence on its own thread: the
/// rebuild first, and — only if that succeeded — [`AppState::swap_conn`].
/// A failed rebuild leaves the live DB untouched, so there is nothing to
/// swap to; a failed swap is still a job error (`status:"error"`), because
/// the DB was rebuilt but this server would keep reading the unlinked
/// pre-rebuild inode until restart (spec §3 "Rebuild connection swap").
/// `result` is `null` on success, matching the CLI's silent success.
/// `pub(crate)` so `maint::doctor`'s `POST /doctor/rebuild` alias can
/// delegate to this exact handler instead of restating the gates and the
/// job (console-api spec §8: "alias + adapt, never duplicate").
pub(crate) async fn rebuild(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let started = Instant::now();
    if let Err(resp) = guard_job("rebuild", &state) {
        return *resp;
    }
    if let Err(e) = split_confirm::<api::rebuild::Request>(body)
        .and_then(|(_req, confirmed)| require_confirm(confirmed))
    {
        return Envelope::err("rebuild", &e, 0);
    }
    let job_state = state.clone();
    let job = jobs::spawn_job(
        state.jobs(),
        state.write_permit().clone(),
        "rebuild",
        true,
        move || {
            let cfg = job_state.cfg();
            let mut ctx = Ctx::lazy(job_state.paths(), &cfg);
            api::rebuild::run(&mut ctx, api::rebuild::Request {})?;
            job_state.swap_conn(job_state.paths())?;
            Ok(Value::Null)
        },
    );
    accepted("rebuild", job, started)
}

/// Canonicalize `repo` and require it inside one of this session's allowed
/// roots (§Security "Path containment"). Nonexistent -> `400`; outside every
/// root -> `403`.
fn contain_repo(state: &AppState, repo: &str) -> Result<PathBuf> {
    let conn = state.conn()?;
    let roots = state.allowed_roots(&conn);
    drop(conn);
    security::contain_abs(&roots, Path::new(repo))
}
