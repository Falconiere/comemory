//! `GET /api/search?q=&limit=` — substring match across kinds.

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

use crate::serve::dto::SearchResponse;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

#[derive(Debug, Deserialize)]
pub struct Params {
    pub q: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    20
}

/// Search the graph by label or id substring.
pub async fn handle(
    State(state): State<ServerState>,
    Query(params): Query<Params>,
) -> Result<Json<SearchResponse>, ApiError> {
    let q = params
        .q
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::invalid_param("missing q"))?;
    if q.len() > 128 {
        return Err(ApiError::invalid_param("q exceeds 128 chars"));
    }
    let limit = params.limit.clamp(1, 100) as usize;
    let graph = state
        .graph
        .lock()
        .map_err(|_| ApiError::graph_error("graph mutex poisoned"))?;
    let results = graph.search_nodes(&q, limit)?;
    Ok(Json(SearchResponse { results }))
}
