//! `GET /api/seed?layer=memory|all` — initial graph payload.

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

use crate::serve::dto::GraphPayload;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

#[derive(Debug, Deserialize)]
pub struct Params {
    #[serde(default = "default_layer")]
    pub layer: String,
}

fn default_layer() -> String {
    "memory".into()
}

/// Return an initial subgraph for the frontend to render on load.
///
/// Query params:
/// - `layer=memory` (default) — memory-layer nodes and edges only.
/// - `layer=all` — memory layer plus code layer (File, Symbol).
/// - Any other value → 400 with `invalid_param`.
pub async fn handle(
    State(state): State<ServerState>,
    Query(params): Query<Params>,
) -> Result<Json<GraphPayload>, ApiError> {
    let graph = state
        .graph
        .lock()
        .map_err(|_| ApiError::graph_error("graph mutex poisoned"))?;
    let payload = match params.layer.as_str() {
        "memory" => graph.seed_memory_layer()?,
        "all" => graph.seed_all()?,
        other => {
            return Err(ApiError::invalid_param(format!("unknown layer: {other}")));
        }
    };
    Ok(Json(payload))
}
