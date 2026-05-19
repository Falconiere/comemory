//! `GET /api/expand?id=<ns:id>&depth=N` — k-hop neighborhood.

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

use crate::serve::dto::GraphPayload;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

#[derive(Debug, Deserialize)]
pub struct Params {
    /// Namespaced node id, e.g. `m:a1b2c3d4`.
    pub id: Option<String>,
    /// Number of hops to expand (1–3). Defaults to 1.
    #[serde(default = "default_depth")]
    pub depth: u32,
}

fn default_depth() -> u32 {
    1
}

/// Expand the graph from a given node id up to `depth` hops.
pub async fn handle(
    State(state): State<ServerState>,
    Query(params): Query<Params>,
) -> Result<Json<GraphPayload>, ApiError> {
    let id = params
        .id
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::invalid_param("missing id"))?;
    if !(1..=3).contains(&params.depth) {
        return Err(ApiError::invalid_param("depth must be 1..=3"));
    }
    let graph = state
        .graph
        .lock()
        .map_err(|_| ApiError::graph_error("graph mutex poisoned"))?;
    let payload = graph.expand_neighbors(&id, params.depth)?;
    if payload.nodes.is_empty() {
        return Err(ApiError::not_found(format!("node {id} not found")));
    }
    Ok(Json(payload))
}
