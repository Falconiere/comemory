//! Stub handler for `GET /api/seed`.

use axum::extract::State;
use axum::Json;

use crate::serve::dto::GraphPayload;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

/// Return an initial subgraph for the frontend to render on load.
pub async fn handle(State(_): State<ServerState>) -> Result<Json<GraphPayload>, ApiError> {
    Err(ApiError::graph_error("not implemented"))
}
