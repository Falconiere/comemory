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
}
