//! `GET /api/v1/graph/nodes`, `GET /api/v1/graph/nodes/{id}`,
//! `GET /api/v1/graph/nodes/{id}/neighbors`, `GET /api/v1/graph/snapshot`,
//! `POST /api/v1/graph/recompute` (console-api spec §5).
//!
//! The four reads are thin transports over [`api::graph_nodes`]; the
//! recompute is a job (`graph-recompute`), because a PageRank pass over
//! every repo is not something to hold an HTTP request open for.
//!
//! `{id}` is ONE percent-encoded path segment: a node id is
//! `file:<repo>:<path>` and its path half contains `/`, so a client sends
//! `file%3Ademo%3Asrc%2Fa.rs` and axum's `Path<String>` hands the decoded
//! id back. An `X-Comemory-Repo` scope ([`RepoScope`]) additionally lets a
//! console pass a bare repo-relative path as the id — see
//! `api::graph_nodes::resolve_node_id`.

use std::time::Instant;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, post};

use crate::api::{self, Ctx};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::jobs;
use crate::serve::routes::{RouteEntry, accepted, guard_job, respond, run_blocking};
use crate::serve::scope::RepoScope;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/graph/nodes",
            command: "graph.nodes",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/graph/snapshot",
            command: "graph.snapshot",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/graph/nodes/{id}",
            command: "graph.node",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/graph/nodes/{id}/neighbors",
            command: "graph.neighbors",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/graph/recompute",
            command: "graph.recompute",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`. The three `/graph/nodes`
/// shapes are registered as distinct patterns; axum 0.8's router matches the
/// literal-vs-parameter split itself, so registration order is irrelevant.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/graph/nodes", get(list))
        .route("/api/v1/graph/snapshot", get(snapshot))
        .route("/api/v1/graph/nodes/{id}", get(node_detail))
        .route("/api/v1/graph/nodes/{id}/neighbors", get(node_neighbors))
        .route("/api/v1/graph/recompute", post(recompute))
}

/// `GET /api/v1/graph/nodes` — page the file nodes (`api::graph_nodes::list`).
async fn list(
    State(state): State<AppState>,
    scope: RepoScope,
    Query(mut req): Query<api::graph_nodes::ListRequest>,
) -> Response {
    scope.fill_if_absent(&mut req.repo);
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::graph_nodes::list(&mut ctx, req)
    })
    .await;
    respond("graph.nodes", result, started)
}

/// `GET /api/v1/graph/snapshot` — the whole capped graph
/// (`api::graph_nodes::snapshot`).
async fn snapshot(
    State(state): State<AppState>,
    scope: RepoScope,
    Query(mut req): Query<api::graph_nodes::SnapshotRequest>,
) -> Response {
    scope.fill_if_absent(&mut req.repo);
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::graph_nodes::snapshot(&mut ctx, req)
    })
    .await;
    respond("graph.snapshot", result, started)
}

/// `GET /api/v1/graph/nodes/{id}` — one node with its top symbols and the
/// memories citing it (`api::graph_nodes::detail`). `404 not_found` when the
/// id names a file with no indexed symbols.
async fn node_detail(
    State(state): State<AppState>,
    scope: RepoScope,
    Path(id): Path<String>,
) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::graph_nodes::detail(&mut ctx, &id, scope.0.as_deref())
    })
    .await;
    respond("graph.node", result, started)
}

/// `GET /api/v1/graph/nodes/{id}/neighbors` — the one-hop file neighborhood
/// (`api::graph_nodes::neighbors`), the same rows `comemory context` reports
/// as a memory's `neighbors` (AC-9).
async fn node_neighbors(
    State(state): State<AppState>,
    scope: RepoScope,
    Path(id): Path<String>,
    Query(req): Query<api::graph_nodes::NeighborsRequest>,
) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::graph_nodes::neighbors(&mut ctx, &id, scope.0.as_deref(), req)
    })
    .await;
    respond("graph.neighbors", result, started)
}

/// `POST /api/v1/graph/recompute` — start a `graph-recompute` job
/// (`api::graph_recompute`). Read-only gate first ([`guard_job`] → `405
/// read_only`); no confirm gate, since a recompute rewrites only derived
/// scores and is idempotent. The request body carries nothing and is
/// ignored.
async fn recompute(State(state): State<AppState>) -> Response {
    let started = Instant::now();
    if let Err(resp) = guard_job("graph.recompute", &state) {
        return *resp;
    }
    let job_state = state.clone();
    let job = jobs::spawn_job(
        state.jobs(),
        state.write_permit().clone(),
        "graph-recompute",
        true,
        move || {
            let cfg = job_state.cfg();
            let mut ctx = Ctx::lazy(job_state.paths(), &cfg);
            let resp = api::graph_recompute::run(&mut ctx, api::graph_recompute::Request {})?;
            serde_json::to_value(resp).map_err(Error::Json)
        },
    );
    accepted("graph.recompute", job, started)
}

#[cfg(test)]
#[path = "tests/graph_nodes.rs"]
mod tests;
