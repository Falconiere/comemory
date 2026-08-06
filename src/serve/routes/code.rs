//! `GET|POST /api/v1/code/search` (`api::search_code`). `GET` takes query
//! params (no vector — a 768-float embedding does not fit in a query
//! string); `POST` takes a JSON body and is vector-capable. Reuses
//! `output::search_code::envelope` so the HTTP `data` payload is
//! byte-identical to the CLI `--json` shape (nested one level deeper). No
//! lazy-reindex trigger over HTTP (spec Non-Goal 8) — `cli::lazy_reindex`
//! never runs on this path.

use std::time::Instant;

use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;

use crate::api::{self, Ctx};
use crate::output::search_code;
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond, run_blocking, track_for};

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
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route(
        "/api/v1/code/search",
        get(code_search_get).post(code_search_post),
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
