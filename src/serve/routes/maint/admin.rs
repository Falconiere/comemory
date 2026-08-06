//! `POST /api/v1/mine` (`api::mine`, no confirm gate — a bounded scan that
//! only mutates when `"apply":true`) and `POST /api/v1/hooks/install`
//! (`api::install_hooks`, confirm-gated, `--repo` contained).

use std::path::{Path, PathBuf};
use std::time::Instant;

use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::Value;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::maint::prune::split_confirm;
use crate::serve::routes::{RouteEntry, guard_mutating, require_confirm, respond, run_blocking};
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
    ]
}

/// This resource's routes, merged into the `maint` resource router.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/mine", post(mine))
        .route("/api/v1/hooks/install", post(hooks_install))
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
        let (req, confirmed) = split_confirm::<api::install_hooks::Request>(body)?;
        contain_repo(&state, &req.repo)?;
        require_confirm(confirmed)?;
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::install_hooks::run(&mut ctx, req)
    })
    .await;
    respond("hooks.install", result, started)
}

/// Canonicalize `repo` and require it inside one of this session's allowed
/// roots (§Security "Path containment"). Nonexistent -> `400`; outside every
/// root -> `403`.
fn contain_repo(state: &AppState, repo: &str) -> crate::prelude::Result<PathBuf> {
    let conn = state.conn()?;
    let roots = state.allowed_roots(&conn);
    drop(conn);
    security::contain_abs(&roots, Path::new(repo))
}
