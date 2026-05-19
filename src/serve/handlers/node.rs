//! Stub handler for `GET /api/node/{id}`.

use axum::extract::State;
use axum::Json;

use crate::serve::dto::NodeDetail;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

/// Return full detail for a single node by namespaced id.
pub async fn handle(State(_): State<ServerState>) -> Result<Json<NodeDetail>, ApiError> {
    Err(ApiError::graph_error("not implemented"))
}
