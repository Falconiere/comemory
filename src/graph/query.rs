//! Read-only graph queries used by the retrieval pipeline.

use kuzu::Value;

use crate::graph::upsert::Graph;
use crate::prelude::*;

impl Graph {
    /// Return the ids of every `Memory` linked to the given repo via `:InRepo`.
    ///
    /// Returns an empty `Vec` when no memories match (including the case where
    /// the `Repo` node itself does not exist yet).
    pub fn neighbors_by_repo(&self, repo: &str) -> Result<Vec<String>> {
        let conn = self.conn()?;
        let cypher = format!(
            "MATCH (m:Memory)-[:InRepo]->(r:Repo {{name: '{}'}}) RETURN m.id",
            crate::graph::upsert::esc(repo),
        );
        let rs = conn
            .query(&cypher)
            .map_err(|e| Error::Other(format!("kuzu query failed: {e}")))?;
        let mut out = Vec::new();
        for row in rs {
            if let Some(Value::String(id)) = row.into_iter().next() {
                out.push(id);
            }
        }
        Ok(out)
    }

    /// Walk the `:Supersedes` chain starting from `start_id`, returning every
    /// `Memory.id` reachable within `depth` hops (1..=depth).
    ///
    /// Depth `0` is clamped to `1` because kuzu's variable-length pattern
    /// requires the lower bound to be at least 1. Returns an empty `Vec` when
    /// the starting memory is missing or has no outgoing supersession edges.
    pub fn supersedes_chain(&self, start_id: &str, depth: u32) -> Result<Vec<String>> {
        let max = depth.max(1);
        let conn = self.conn()?;
        let cypher = format!(
            "MATCH (m:Memory {{id: '{id}'}})-[:Supersedes*1..{max}]->(n:Memory) RETURN n.id",
            id = crate::graph::upsert::esc(start_id),
            max = max,
        );
        let rs = conn
            .query(&cypher)
            .map_err(|e| Error::Other(format!("kuzu query failed: {e}")))?;
        let mut out = Vec::new();
        for row in rs {
            if let Some(Value::String(id)) = row.into_iter().next() {
                out.push(id);
            }
        }
        Ok(out)
    }

    /// Return the ids of every `Memory` linked to `id` via an outgoing
    /// `:ConflictsWith` edge. Empty `Vec` when there are no conflicts.
    pub fn conflicts_of(&self, id: &str) -> Result<Vec<String>> {
        let conn = self.conn()?;
        let cypher = format!(
            "MATCH (m:Memory {{id: '{id}'}})-[:ConflictsWith]->(n:Memory) RETURN n.id",
            id = crate::graph::upsert::esc(id),
        );
        let rs = conn
            .query(&cypher)
            .map_err(|e| Error::Other(format!("kuzu query failed: {e}")))?;
        let mut out = Vec::new();
        for row in rs {
            if let Some(Value::String(other)) = row.into_iter().next() {
                out.push(other);
            }
        }
        Ok(out)
    }
}
