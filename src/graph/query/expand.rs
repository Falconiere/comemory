//! `Graph::expand_neighbors` — BFS neighborhood expansion over the full graph.

use std::collections::BTreeSet;

use crate::graph::upsert::Graph;
use crate::prelude::*;
use crate::serve::dto::GraphPayload;

impl Graph {
    /// Return nodes/edges within `depth` hops of `ns_id` (namespaced id).
    ///
    /// Uses BFS over the full `seed_all` payload so the walk is portable
    /// across kuzu versions. Depth is clamped to at least 1.
    /// Returns an empty [`GraphPayload`] when `ns_id` is not in the graph.
    pub fn expand_neighbors(&self, ns_id: &str, depth: u32) -> Result<GraphPayload> {
        let full = self.seed_all()?;
        if !full.nodes.iter().any(|n| n.id == ns_id) {
            return Ok(GraphPayload::default());
        }

        let mut reachable: BTreeSet<String> = BTreeSet::new();
        reachable.insert(ns_id.to_string());
        for _ in 0..depth.max(1) {
            let mut frontier = BTreeSet::new();
            for e in &full.edges {
                if reachable.contains(&e.source) && !reachable.contains(&e.target) {
                    frontier.insert(e.target.clone());
                } else if reachable.contains(&e.target) && !reachable.contains(&e.source) {
                    frontier.insert(e.source.clone());
                }
            }
            if frontier.is_empty() {
                break;
            }
            reachable.extend(frontier);
        }

        let nodes = full
            .nodes
            .into_iter()
            .filter(|n| reachable.contains(&n.id))
            .collect();
        let edges = full
            .edges
            .into_iter()
            .filter(|e| reachable.contains(&e.source) && reachable.contains(&e.target))
            .collect();

        Ok(GraphPayload { nodes, edges })
    }
}
