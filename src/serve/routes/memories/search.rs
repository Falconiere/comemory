//! `GET|POST /api/v1/memories/search` (`api::search`) and
//! `GET|POST /api/v1/context` (`api::context`). `GET` takes query params
//! (no vector — a 1024-float embedding does not fit in a query string);
//! `POST` takes a JSON body and is vector-capable. Both reuse the exact
//! `output::{search,context}::envelope` builders the CLI's `--json` path
//! calls, so the HTTP `data` payload is byte-identical to the CLI shape
//! (just nested one level deeper, inside the `/api/v1` envelope).

use std::time::Instant;

use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;

use crate::api::{self, Ctx};
use crate::output::search::ScopeEcho;
use crate::output::{context, search};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::{respond, run_blocking};

/// This module's routes, merged into the `memories` resource router.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/memories/search",
            get(memories_search_get).post(memories_search_post),
        )
        .route("/api/v1/context", get(context_get).post(context_post))
}

async fn memories_search_get(
    State(state): State<AppState>,
    Query(req): Query<api::search::Request>,
) -> Response {
    handle("search", state, move |state| run_search(state, req)).await
}

async fn memories_search_post(
    State(state): State<AppState>,
    Json(req): Json<api::search::Request>,
) -> Response {
    handle("search", state, move |state| run_search(state, req)).await
}

async fn context_get(
    State(state): State<AppState>,
    Query(req): Query<api::context::Request>,
) -> Response {
    handle("context", state, move |state| run_context(state, req)).await
}

async fn context_post(
    State(state): State<AppState>,
    Json(req): Json<api::context::Request>,
) -> Response {
    handle("context", state, move |state| run_context(state, req)).await
}

/// Shared spawn-blocking + envelope wiring for the four handlers above.
async fn handle<F>(command: &'static str, state: AppState, f: F) -> Response
where
    F: FnOnce(AppState) -> Result<Value> + Send + 'static,
{
    let started = Instant::now();
    let result = run_blocking(move || f(state)).await;
    respond(command, result, started)
}

/// `track` mirrors §Security's read-only side-effect degradation: a
/// read-only server never writes access counts / `retrieval_log`, on top of
/// the same `COMEMORY_DISABLE_ACCESS_TRACKING` test hook the CLI honors (so
/// AC-2's cross-surface comparison stays reorder-free on both sides).
fn track_for(state: &AppState) -> Result<bool> {
    Ok(!state.read_only() && crate::cli::track_searches()?)
}

fn run_search(state: AppState, req: api::search::Request) -> Result<Value> {
    let track = track_for(&state)?;
    let cfg = state.cfg();
    let mut conn = state.conn()?;
    let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
    let result = api::search::run(&mut ctx, req, track)?;
    let envelope = search::envelope(
        &result.hits,
        result.query_id.as_deref(),
        result.meta,
        &result.nav,
        state.paths().data_dir(),
        ScopeEcho::of(&result.scope),
    );
    serde_json::to_value(envelope).map_err(Error::Json)
}

fn run_context(state: AppState, req: api::context::Request) -> Result<Value> {
    let track = track_for(&state)?;
    let cfg = state.cfg();
    let mut conn = state.conn()?;
    let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
    let result = api::context::run(&mut ctx, req, track)?;
    let envelope = context::envelope(
        &result.bundle,
        result.query_id.as_deref(),
        result.meta,
        ScopeEcho::of(&result.scope),
    );
    serde_json::to_value(envelope).map_err(Error::Json)
}
