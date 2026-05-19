//! Stub handler for `GET /api/expand`.

use axum::extract::State;
use axum::Json;

use crate::serve::dto::GraphPayload;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

/// Expand the graph from a given node id.
pub async fn handle(State(_): State<ServerState>) -> Result<Json<GraphPayload>, ApiError> {
    Err(ApiError::graph_error("not implemented"))
}
