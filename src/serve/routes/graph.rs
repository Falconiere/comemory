//! `GET /api/v1/graph` (`api::graph`) and `GET /api/v1/edges` (`api::edges`).
//!
//! The graph route reuses the exact `build_code_graph` / `build_graph_page`
//! pair the legacy `GET /api/graph` handler already calls (via
//! `api::graph::run`) — no second query path — and keeps the same
//! backward-compatible full-vs-page switch (absent `limit` **and** `offset`
//! → the whole graph; either present → a windowed page). The edges route
//! skips the one-time `edge_fts` self-heal on a read-only server
//! (§Security).

use std::time::Instant;

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use serde_json::Value;

use crate::api::{self, Ctx};
use crate::output::edges as edges_output;
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond, run_blocking};

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/graph",
            command: "graph",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/edges",
            command: "edges",
            mutating: false,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/graph", get(graph))
        .route("/api/v1/edges", get(edges))
}

/// `GET /api/v1/graph` — the full graph or a windowed page (`api::graph`).
async fn graph(
    State(state): State<AppState>,
    scope: crate::serve::scope::RepoScope,
    Query(mut req): Query<api::graph::Request>,
) -> Response {
    scope.apply(&mut req.repo);
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::graph::run(&mut ctx, req)
    })
    .await;
    respond("graph", result, started)
}

/// `GET /api/v1/edges` — paged lexical search over the relation graph
/// (`api::edges`). The `edge_fts` self-heal is allowed only when the server
/// is not read-only.
async fn edges(State(state): State<AppState>, Query(req): Query<api::edges::Request>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || run_edges(state, req)).await;
    respond("edges", result, started)
}

fn run_edges(state: AppState, req: api::edges::Request) -> Result<Value> {
    let allow_self_heal = !state.read_only();
    let cfg = state.cfg();
    let mut conn = state.conn()?;
    let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
    let result = api::edges::run(&mut ctx, req, allow_self_heal)?;
    let envelope =
        edges_output::envelope(&result.hits, result.limit, result.offset, result.has_more);
    serde_json::to_value(envelope).map_err(Error::Json)
}
