//! `Graph::search_nodes` — case-insensitive substring search across all node kinds.

use crate::graph::upsert::Graph;
use crate::prelude::*;
use crate::serve::dto::SearchResult;

impl Graph {
    /// Case-insensitive substring match across `Memory.id`, `Tag.name`,
    /// `Author.name`, `Repo.name`, `Symbol.name`, `File.path`.
    pub fn search_nodes(&self, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
        if q.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let needle = q.to_lowercase();
        let conn = self.conn()?;
        let mut hits: Vec<SearchResult> = Vec::new();

        let kinds: [(&str, &str, &str, &str); 6] = [
            ("Memory", "id", "m", "id"),
            ("Tag", "name", "t", "name"),
            ("Author", "name", "a", "name"),
            ("Repo", "name", "r", "name"),
            ("Symbol", "name", "s", "qualified"),
            ("File", "path", "f", "qualified"),
        ];
        for (label, match_field, ns, id_field) in kinds {
            // Alias both columns so kuzu never sees duplicate column names
            // (Memory has match_field == id_field == "id").
            let cypher = format!(
                "MATCH (n:{label}) RETURN n.{match_field} AS match_val, n.{id_field} AS id_val"
            );
            let rs = conn
                .query(&cypher)
                .map_err(|e| Error::Other(format!("kuzu query failed: {e}")))?;
            for row in rs {
                let match_val = match row.first() {
                    Some(kuzu::Value::String(s)) => s.clone(),
                    _ => continue,
                };
                let id_val = match row.get(1) {
                    Some(kuzu::Value::String(s)) => s.clone(),
                    _ => continue,
                };
                if !match_val.to_lowercase().contains(&needle) {
                    continue;
                }
                hits.push(SearchResult {
                    id: format!("{ns}:{id_val}"),
                    label: match_val,
                    kind: label.to_string(),
                });
                if hits.len() >= limit {
                    return Ok(hits);
                }
            }
        }
        Ok(hits)
    }
}
