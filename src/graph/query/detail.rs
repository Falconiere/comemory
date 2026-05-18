//! `Graph::node_detail` — single-node lookup with directional edge refs.

use crate::graph::upsert::Graph;
use crate::prelude::*;
use crate::serve::dto::{EdgeRef, NodeDetail};

impl Graph {
    /// Resolve a single namespaced id to its [`NodeDetail`]. Returns
    /// `Ok(None)` when the node is missing.
    pub fn node_detail(&self, ns_id: &str) -> Result<Option<NodeDetail>> {
        let full = self.seed_all()?;
        let Some(node) = full.nodes.iter().find(|n| n.id == ns_id).cloned() else {
            return Ok(None);
        };

        let outbound = full
            .edges
            .iter()
            .filter(|e| e.source == ns_id)
            .map(|e| EdgeRef {
                edge_kind: e.kind.clone(),
                target: Some(e.target.clone()),
                source: None,
            })
            .collect();
        let inbound = full
            .edges
            .iter()
            .filter(|e| e.target == ns_id)
            .map(|e| EdgeRef {
                edge_kind: e.kind.clone(),
                target: None,
                source: Some(e.source.clone()),
            })
            .collect();

        Ok(Some(NodeDetail {
            node,
            memory_body: None,
            frontmatter: None,
            outbound,
            inbound,
        }))
    }
}
