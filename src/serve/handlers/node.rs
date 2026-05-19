//! `GET /api/node/{ns:id}` — full detail for a single node.

use axum::extract::{Path, State};
use axum::Json;

use crate::errors::Error;
use crate::memory::MemoryStore;
use crate::serve::dto::NodeDetail;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

/// Return full detail for a single node by namespaced id.
pub async fn handle(
    State(state): State<ServerState>,
    Path(ns_id): Path<String>,
) -> Result<Json<NodeDetail>, ApiError> {
    let graph = state
        .graph
        .lock()
        .map_err(|_| ApiError::graph_error("graph mutex poisoned"))?;
    let mut detail = graph
        .node_detail(&ns_id)?
        .ok_or_else(|| ApiError::not_found(format!("node {ns_id} not found")))?;
    drop(graph);

    if detail.node.kind == "Memory" {
        if let Some(raw_id) = ns_id.strip_prefix("m:") {
            let store = MemoryStore::new((*state.paths).clone());
            let rec = store.load(raw_id)?;
            detail.memory_body = Some(rec.body);
            detail.frontmatter = Some(serde_json::to_value(rec.frontmatter).map_err(Error::Json)?);
        }
    }

    Ok(Json(detail))
}
