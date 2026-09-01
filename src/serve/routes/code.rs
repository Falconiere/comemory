//! `GET|POST /api/v1/code/search` (`api::search_code`, `GET` no vector,
//! `POST` vector-capable; no lazy-reindex over HTTP, spec Non-Goal 8).
//! `POST /api/v1/code/ast` (`api::ast`): a read, but `req.file` needs
//! containment first. `POST /api/v1/code/index` (`api::index_code`, job)
//! and `POST /api/v1/code/ingest` (`api::ingest_code`, job, NDJSON body,
//! its own 64 MiB per-route limit) round out this resource.

use std::path::Path;
use std::time::Instant;

use axum::extract::{DefaultBodyLimit, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;

use crate::api::{self, Ctx};
use crate::output::search_code;
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::envelope::Envelope;
use crate::serve::jobs;
use crate::serve::routes::{RouteEntry, accepted, guard_job, respond, run_blocking, track_for};
use crate::serve::security;

/// The per-route body limit `POST /api/v1/code/ingest` carries — well above
/// the 5 MiB global default (`fileio::MAX_FILE_BYTES`), since a real NDJSON
/// batch of embedded symbol rows runs large (§Security "Body limits").
const INGEST_BODY_LIMIT: usize = 64 * 1024 * 1024;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/code/search",
            command: "search-code",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/code/search",
            command: "search-code",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/code/ast",
            command: "ast",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/code/index",
            command: "index-code",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/code/ingest",
            command: "ingest-code",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/code/search",
            get(code_search_get).post(code_search_post),
        )
        .route("/api/v1/code/ast", post(code_ast))
        .route("/api/v1/code/index", post(code_index))
        .route(
            "/api/v1/code/ingest",
            post(code_ingest).layer(DefaultBodyLimit::max(INGEST_BODY_LIMIT)),
        )
}

async fn code_search_get(
    State(state): State<AppState>,
    Query(req): Query<api::search_code::Request>,
) -> Response {
    handle(state, req).await
}

async fn code_search_post(
    State(state): State<AppState>,
    Json(req): Json<api::search_code::Request>,
) -> Response {
    handle(state, req).await
}

/// Shared spawn-blocking + envelope wiring for the two handlers above.
async fn handle(state: AppState, req: api::search_code::Request) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || run(state, req)).await;
    respond("code.search", result, started)
}

fn run(state: AppState, req: api::search_code::Request) -> Result<Value> {
    let track = track_for(&state)?;
    let cfg = state.cfg();
    let mut conn = state.conn()?;
    let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
    let result = api::search_code::run(&mut ctx, req, track)?;
    let envelope = search_code::envelope(&result.hits, result.query_id.as_deref(), result.meta);
    serde_json::to_value(envelope).map_err(Error::Json)
}

/// `POST /api/v1/code/ast` — run an ast-grep pattern against a file
/// (`api::ast`), read-only but requiring `req.file` inside an allowed root
/// before the filesystem read (§Security "Path containment"). Nonexistent
/// path -> `400`; outside every root -> `403`.
async fn code_ast(
    State(state): State<AppState>,
    Json(mut req): Json<api::ast::Request>,
) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let conn = state.conn()?;
        let roots = state.allowed_roots(&conn);
        drop(conn);
        let canonical = security::contain_abs(&roots, Path::new(&req.file))?;
        req.file = canonical.to_string_lossy().into_owned();
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::ast::run(&mut ctx, req)
    })
    .await;
    respond("code.ast", result, started)
}

/// `POST /api/v1/code/index` — start an `index-code` job (`api::index_code`)
/// over `req.path`, contained to an allowed root BEFORE the job is created
/// (AC-7: an out-of-root or nonexistent path never spawns a job).
/// `405 read_only` on a `--read-only` server; never `503 busy` — a
/// job-creating `POST` always answers `202` immediately ([`guard_job`]).
/// The job reports progress and a log tail into the registry via
/// `jobs::worker::RegistryProgressSink` (AC-33), streamed as the SSE
/// `progress` event alongside the unchanged `status` events (AC-34).
async fn code_index(
    State(state): State<AppState>,
    Json(mut req): Json<api::index_code::Request>,
) -> Response {
    let started = Instant::now();
    if let Err(resp) = guard_job("index-code", &state) {
        return *resp;
    }
    let contain_state = state.clone();
    let contained = run_blocking(move || -> Result<api::index_code::Request> {
        let conn = contain_state.conn()?;
        let roots = contain_state.allowed_roots(&conn);
        drop(conn);
        let canonical = security::contain_abs(&roots, Path::new(&req.path))?;
        req.path = canonical.to_string_lossy().into_owned();
        Ok(req)
    })
    .await;
    let req = match contained {
        Ok(req) => req,
        Err(e) => return Envelope::err("index-code", &e, 0),
    };
    let job_state = state.clone();
    let job = jobs::spawn_job_with_id(
        state.jobs(),
        state.write_permit().clone(),
        "index-code",
        true,
        move |job_id| {
            let cfg = job_state.cfg();
            let mut ctx = Ctx::lazy(job_state.paths(), &cfg);
            let sink = jobs::worker::RegistryProgressSink::new(job_state.jobs().clone(), job_id);
            let resp = api::index_code::run_with_progress(&mut ctx, req, Some(&sink))?;
            serde_json::to_value(resp).map_err(Error::Json)
        },
    );
    accepted("index-code", job, started)
}

/// `POST /api/v1/code/ingest` — start an `ingest-code` job (`api::ingest_code`)
/// over the raw NDJSON request body (bounded by [`INGEST_BODY_LIMIT`], not
/// the global 5 MiB default). `405 read_only` on a `--read-only` server.
async fn code_ingest(State(state): State<AppState>, body: String) -> Response {
    let started = Instant::now();
    if let Err(resp) = guard_job("ingest-code", &state) {
        return *resp;
    }
    let job_state = state.clone();
    let job = jobs::spawn_job(
        state.jobs(),
        state.write_permit().clone(),
        "ingest-code",
        true,
        move || {
            let cfg = job_state.cfg();
            let mut ctx = Ctx::lazy(job_state.paths(), &cfg);
            let resp = api::ingest_code::run(&mut ctx, &body)?;
            serde_json::to_value(resp).map_err(Error::Json)
        },
    );
    accepted("ingest-code", job, started)
}
