//! `GET /api/v1/memory-stores`, `GET /api/v1/memory-stores/{id}`,
//! `POST /api/v1/memory-stores` and `POST /api/v1/memory-stores/{id}/sync`
//! (console-api spec §10) — the console's view of the ONE memory store
//! comemory models (`api::memory_store`).
//!
//! `POST /memory-stores` is registered as a mutating route even though it
//! can only ever answer `501 unsupported` (spec Non-Goal 3): it goes through
//! [`guard_mutating`] first, so a `--read-only` server still answers `405
//! read_only` — read-only outranks every other refusal (AC-19), and a client
//! must not be able to tell the two servers apart by which "no" they get.
//!
//! The sync route is job-backed (`store-sync`): it shells out to `git`, whose
//! runtime is a network round trip, so it answers `202 Accepted` immediately
//! and streams each `git` step into the job's log through
//! [`crate::serve::jobs::Registry::push_log`].

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::jobs;
use crate::serve::routes::{
    RouteEntry, accepted, guard_job, guard_mutating, respond, run_blocking,
};

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/memory-stores",
            command: "memory-stores",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/memory-stores/{id}",
            command: "memory-stores",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/memory-stores",
            command: "memory-stores.create",
            mutating: true,
        },
        RouteEntry {
            method: "PATCH",
            path: "/memory-stores/{id}",
            command: "memory-stores.patch",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/memory-stores/{id}/sync",
            command: "memory-stores.sync",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/memory-stores", get(list).post(create))
        .route("/api/v1/memory-stores/{id}", get(get_one).patch(patch))
        .route("/api/v1/memory-stores/{id}/sync", post(sync))
}

/// `GET /api/v1/memory-stores` — the one store, as a one-element array so a
/// console can render a list without special-casing the count.
async fn list(State(state): State<AppState>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::memory_store::list(&mut ctx)
    })
    .await;
    respond("memory-stores", result, started)
}

/// `GET /api/v1/memory-stores/{id}` — that same store by id; anything other
/// than `default` is `404 not_found`.
async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::memory_store::get(&mut ctx, &id)
    })
    .await;
    respond("memory-stores", result, started)
}

/// `PATCH /api/v1/memory-stores/{id}` — rewrite the supplied `[git]` keys in
/// `config.toml` (`api::memory_store::patch`) and answer the updated store.
///
/// Read-only-gated via [`guard_mutating`]; NOT confirm-gated: the write is two
/// scalar knobs in a config file, reversible by the inverse PATCH, unlike the
/// confirm-gated routes that destroy data. `AppState.cfg` is reloaded inside
/// the same blocking closure, and the body is then rendered from the RELOADED
/// config through `api::memory_store::get` (the same shape `routes::config`
/// uses) rather than from the patch's own file-only view — so an env override
/// the reload re-applies (`COMEMORY_GIT_AUTO_SYNC`) shows in the response
/// exactly as the very next `GET` reports it; the body and the server's live
/// config cannot disagree.
async fn patch(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<api::memory_store::PatchRequest>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("memory-stores.patch", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        {
            let cfg = state.cfg();
            let mut ctx = Ctx::lazy(state.paths(), &cfg);
            api::memory_store::patch(&mut ctx, &id, &req)?;
        }
        state.reload_cfg(state.paths())?;
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::memory_store::get(&mut ctx, &id)
    })
    .await;
    respond("memory-stores.patch", result, started)
}

/// `POST /api/v1/memory-stores` — always `501 unsupported` (spec Non-Goal 3),
/// after the read-only gate. The body is still typed and
/// `deny_unknown_fields`-checked so the refusal is about the model rather
/// than about a typo in the request.
async fn create(
    State(state): State<AppState>,
    Json(req): Json<api::memory_store::CreateRequest>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("memory-stores.create", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        api::memory_store::create(req)
    })
    .await;
    respond("memory-stores.create", result, started)
}

/// `POST /api/v1/memory-stores/{id}/sync` — start the `store-sync` job
/// (`api::memory_store::sync`). The body is optional; `{"push": true}` pushes
/// once without flipping `[git] auto_sync`.
///
/// Gate order: [`guard_job`] first (`405 read_only`, never `503 busy` — a
/// job-creating `POST` answers immediately and waits for the write permit
/// inside the job). Not confirm-gated: a pull-commit-push of the markdown
/// source of truth is the same work `[git] auto_sync` does on every save.
async fn sync(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<api::memory_store::SyncRequest>>,
) -> Response {
    let started = Instant::now();
    if let Err(resp) = guard_job("memory-stores.sync", &state) {
        return *resp;
    }
    let req = body.map_or_else(api::memory_store::SyncRequest::default, |Json(r)| r);
    let registry = Arc::clone(state.jobs());
    let job_state = state.clone();
    let job = jobs::spawn_job_with_id(
        state.jobs(),
        Arc::clone(state.write_permit()),
        "store-sync",
        true,
        move |job_id| {
            let cfg = job_state.cfg();
            let mut ctx = Ctx::lazy(job_state.paths(), &cfg);
            let done = api::memory_store::sync(&mut ctx, &id, &req, |line| {
                if let Err(e) = registry.push_log(&job_id, line.to_string()) {
                    tracing::debug!(job_id = %job_id, error = %e, "store-sync log line dropped");
                }
            })?;
            Ok(serde_json::to_value(done)?)
        },
    );
    accepted("memory-stores.sync", job, started)
}

#[cfg(test)]
#[path = "tests/memory_stores.rs"]
mod tests;
