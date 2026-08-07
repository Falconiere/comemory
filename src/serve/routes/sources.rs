//! `GET /api/v1/sources` (`api::sources`). The `reconcile` side effect is
//! computed server-side from the read-only flag — never taken from the
//! query string, so a client cannot force a mirror write on a read-only
//! server (§Security "Read-only side-effect degradation").
//!
//! `POST /api/v1/sources` (`api::index`, job) registers + reconciles one or
//! more sources, every `req.path` entry contained to an allowed root
//! BEFORE the job is created (AC-7).
//!
//! `DELETE /api/v1/sources?target=<id|path>&confirm=true` (`api::unindex`)
//! also lives here — the query form expresses both the id and the
//! registered-path target, mirroring `comemory unindex <SOURCE_ID|PATH>`.

use std::path::Path;
use std::time::Instant;

use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::{self, Ctx};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::envelope::Envelope;
use crate::serve::jobs;
use crate::serve::routes::{
    RouteEntry, accepted, guard_job, guard_mutating, require_confirm, respond, run_blocking,
};
use crate::serve::security;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/sources",
            command: "sources",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/sources",
            command: "index",
            mutating: true,
        },
        RouteEntry {
            method: "DELETE",
            path: "/sources",
            command: "unindex",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route(
        "/api/v1/sources",
        get(sources).post(index_sources).delete(unindex),
    )
}

/// `GET /api/v1/sources` — list registered sources (`api::sources`). The
/// mirror reconcile runs only when the server is not `--read-only`.
async fn sources(State(state): State<AppState>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let req = api::sources::Request {
            reconcile: !state.read_only(),
        };
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::sources::run(&mut ctx, req)
    })
    .await;
    respond("sources", result, started)
}

/// `POST /api/v1/sources` — start an `index` job (`api::index`) registering
/// and reconciling every `req.path` entry, each contained to an allowed
/// root before the job is created.
async fn index_sources(
    State(state): State<AppState>,
    Json(mut req): Json<api::index::Request>,
) -> Response {
    let started = Instant::now();
    if let Err(resp) = guard_job("index", &state) {
        return *resp;
    }
    let contain_state = state.clone();
    let contained = run_blocking(move || -> Result<api::index::Request> {
        let conn = contain_state.conn()?;
        let roots = contain_state.allowed_roots(&conn);
        drop(conn);
        for p in &mut req.path {
            let canonical = security::contain_abs(&roots, Path::new(p.as_str()))?;
            *p = canonical.to_string_lossy().into_owned();
        }
        Ok(req)
    })
    .await;
    let req = match contained {
        Ok(req) => req,
        Err(e) => return Envelope::err("index", &e, 0),
    };
    let job_state = state.clone();
    let job = jobs::spawn_job(
        state.jobs(),
        state.write_permit().clone(),
        "index",
        true,
        move || {
            let cfg = job_state.cfg();
            let mut ctx = Ctx::lazy(job_state.paths(), &cfg);
            let resp = api::index::run(&mut ctx, req)?;
            serde_json::to_value(resp).map_err(Error::Json)
        },
    );
    accepted("index", job, started)
}

/// `?target=&confirm=true` on `DELETE /sources` — transport-level; not part
/// of `api::unindex::Request` since the CLI has no `--confirm` concept.
#[derive(Deserialize)]
struct UnindexQuery {
    target: String,
    #[serde(default)]
    confirm: bool,
}

/// `DELETE /api/v1/sources?target=&confirm=true` — unregister a source
/// (`api::unindex`), confirm gated. `guard_mutating` (read-only/busy) runs
/// before [`require_confirm`] so a read-only server rejects with `405` even
/// without `?confirm=true` (AC-19).
async fn unindex(State(state): State<AppState>, Query(query): Query<UnindexQuery>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("unindex", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        require_confirm(query.confirm)?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::unindex::run(
            &mut ctx,
            api::unindex::Request {
                target: query.target,
            },
        )
    })
    .await;
    respond("unindex", result, started)
}
