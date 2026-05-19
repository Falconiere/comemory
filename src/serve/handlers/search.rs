//! Stub handler for `GET /api/search`.

use axum::extract::State;
use axum::Json;

use crate::serve::dto::SearchResponse;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

/// Search the graph by label or id prefix.
pub async fn handle(State(_): State<ServerState>) -> Result<Json<SearchResponse>, ApiError> {
    Err(ApiError::graph_error("not implemented"))
}
