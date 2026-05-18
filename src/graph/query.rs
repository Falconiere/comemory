//! Read-only graph queries used by the retrieval pipeline.

use std::collections::BTreeSet;

use kuzu::Value;
use serde_json::json;

use crate::graph::upsert::Graph;
use crate::prelude::*;
use crate::serve::dto::{edge_id, EdgeDto, GraphPayload, NodeDto};

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

    /// Return the full memory-layer subgraph: `Memory`, `Repo`, `Author`,
    /// `Tag` nodes plus memory-layer edges (`InRepo`, `AuthoredBy`,
    /// `Tagged`, `Supersedes`, `ConflictsWith`, `RelatesTo`, `DerivedFrom`).
    pub fn seed_memory_layer(&self) -> Result<GraphPayload> {
        let conn = self.conn()?;
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        push_nodes(
            &conn,
            "MATCH (m:Memory) RETURN m.id, m.kind, m.created, m.quality",
            |row| {
                let id = string(&row, 0)?;
                let kind = string(&row, 1)?;
                let created = string(&row, 2)?;
                let quality = int64(&row, 3)?;
                Some(NodeDto {
                    id: format!("m:{id}"),
                    label: id.clone(),
                    kind: "Memory".into(),
                    props: json!({
                        "memory_kind": kind,
                        "created": created,
                        "quality": quality,
                    }),
                })
            },
            &mut nodes,
        )?;
        push_nodes(
            &conn,
            "MATCH (r:Repo) RETURN r.name",
            |row| {
                let n = string(&row, 0)?;
                Some(NodeDto {
                    id: format!("r:{n}"),
                    label: n,
                    kind: "Repo".into(),
                    props: json!({}),
                })
            },
            &mut nodes,
        )?;
        push_nodes(
            &conn,
            "MATCH (a:Author) RETURN a.name",
            |row| {
                let n = string(&row, 0)?;
                Some(NodeDto {
                    id: format!("a:{n}"),
                    label: n,
                    kind: "Author".into(),
                    props: json!({}),
                })
            },
            &mut nodes,
        )?;
        push_nodes(
            &conn,
            "MATCH (t:Tag) RETURN t.name",
            |row| {
                let n = string(&row, 0)?;
                Some(NodeDto {
                    id: format!("t:{n}"),
                    label: n,
                    kind: "Tag".into(),
                    props: json!({}),
                })
            },
            &mut nodes,
        )?;

        push_edges_memory_layer(&conn, &mut edges)?;

        Ok(GraphPayload { nodes, edges })
    }

    /// Memory layer plus code layer (`File`, `Symbol` nodes plus
    /// `DefinedIn`, `Calls`, `Imports`, `ReferencesFile`,
    /// `ReferencesSymbol` edges).
    pub fn seed_all(&self) -> Result<GraphPayload> {
        let mut payload = self.seed_memory_layer()?;
        let conn = self.conn()?;

        push_nodes(
            &conn,
            "MATCH (f:File) RETURN f.qualified, f.repo, f.path",
            |row| {
                let q = string(&row, 0)?;
                let repo = string(&row, 1)?;
                let path = string(&row, 2)?;
                Some(NodeDto {
                    id: format!("f:{q}"),
                    label: path.clone(),
                    kind: "File".into(),
                    props: json!({ "repo": repo, "path": path }),
                })
            },
            &mut payload.nodes,
        )?;
        push_nodes(
            &conn,
            "MATCH (s:Symbol) RETURN s.qualified, s.name, s.kind, s.language",
            |row| {
                let q = string(&row, 0)?;
                let name = string(&row, 1)?;
                let sk = string(&row, 2)?;
                let lang = string(&row, 3)?;
                Some(NodeDto {
                    id: format!("s:{q}"),
                    label: name.clone(),
                    kind: "Symbol".into(),
                    props: json!({ "name": name, "symbol_kind": sk, "language": lang }),
                })
            },
            &mut payload.nodes,
        )?;

        push_edges(
            &conn,
            "MATCH (s:Symbol)-[:DefinedIn]->(f:File) RETURN s.qualified, f.qualified",
            |a, b| (format!("s:{a}"), "DefinedIn".to_string(), format!("f:{b}")),
            &mut payload.edges,
        )?;
        push_edges(
            &conn,
            "MATCH (s:Symbol)-[:Calls]->(t:Symbol) RETURN s.qualified, t.qualified",
            |a, b| (format!("s:{a}"), "Calls".to_string(), format!("s:{b}")),
            &mut payload.edges,
        )?;
        push_edges(
            &conn,
            "MATCH (a:File)-[:Imports]->(b:File) RETURN a.qualified, b.qualified",
            |a, b| (format!("f:{a}"), "Imports".to_string(), format!("f:{b}")),
            &mut payload.edges,
        )?;
        push_edges(
            &conn,
            "MATCH (m:Memory)-[:ReferencesFile]->(f:File) RETURN m.id, f.qualified",
            |a, b| {
                (
                    format!("m:{a}"),
                    "ReferencesFile".to_string(),
                    format!("f:{b}"),
                )
            },
            &mut payload.edges,
        )?;
        push_edges(
            &conn,
            "MATCH (m:Memory)-[:ReferencesSymbol]->(s:Symbol) RETURN m.id, s.qualified",
            |a, b| {
                (
                    format!("m:{a}"),
                    "ReferencesSymbol".to_string(),
                    format!("s:{b}"),
                )
            },
            &mut payload.edges,
        )?;

        Ok(payload)
    }

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

/// Push all memory-layer edges into `edges`.
pub fn push_edges_memory_layer(
    conn: &kuzu::Connection<'_>,
    edges: &mut Vec<EdgeDto>,
) -> Result<()> {
    push_edges(
        conn,
        "MATCH (m:Memory)-[:InRepo]->(r:Repo) RETURN m.id, r.name",
        |a, b| (format!("m:{a}"), "InRepo".to_string(), format!("r:{b}")),
        edges,
    )?;
    push_edges(
        conn,
        "MATCH (m:Memory)-[:AuthoredBy]->(a:Author) RETURN m.id, a.name",
        |a, b| (format!("m:{a}"), "AuthoredBy".to_string(), format!("a:{b}")),
        edges,
    )?;
    push_edges(
        conn,
        "MATCH (m:Memory)-[:Tagged]->(t:Tag) RETURN m.id, t.name",
        |a, b| (format!("m:{a}"), "Tagged".to_string(), format!("t:{b}")),
        edges,
    )?;
    push_edges(
        conn,
        "MATCH (m:Memory)-[:Supersedes]->(n:Memory) RETURN m.id, n.id",
        |a, b| (format!("m:{a}"), "Supersedes".to_string(), format!("m:{b}")),
        edges,
    )?;
    push_edges(
        conn,
        "MATCH (m:Memory)-[:ConflictsWith]->(n:Memory) RETURN m.id, n.id",
        |a, b| {
            (
                format!("m:{a}"),
                "ConflictsWith".to_string(),
                format!("m:{b}"),
            )
        },
        edges,
    )?;
    push_edges(
        conn,
        "MATCH (m:Memory)-[:RelatesTo]->(n:Memory) RETURN m.id, n.id",
        |a, b| (format!("m:{a}"), "RelatesTo".to_string(), format!("m:{b}")),
        edges,
    )?;
    push_edges(
        conn,
        "MATCH (m:Memory)-[:DerivedFrom]->(n:Memory) RETURN m.id, n.id",
        |a, b| {
            (
                format!("m:{a}"),
                "DerivedFrom".to_string(),
                format!("m:{b}"),
            )
        },
        edges,
    )?;
    Ok(())
}

/// Execute `cypher`, build a [`NodeDto`] from each row via `build`, and
/// append results to `out`. Rows for which `build` returns `None` are skipped.
pub fn push_nodes<F>(
    conn: &kuzu::Connection<'_>,
    cypher: &str,
    mut build: F,
    out: &mut Vec<NodeDto>,
) -> Result<()>
where
    F: FnMut(Vec<kuzu::Value>) -> Option<NodeDto>,
{
    let rs = conn
        .query(cypher)
        .map_err(|e| Error::Other(format!("kuzu query failed: {e}")))?;
    for row in rs {
        if let Some(n) = build(row) {
            out.push(n);
        }
    }
    Ok(())
}

/// Execute `cypher` (returns two string columns), build an [`EdgeDto`] via
/// `build`, and append to `out`. Rows with non-string values are skipped.
pub fn push_edges<F>(
    conn: &kuzu::Connection<'_>,
    cypher: &str,
    mut build: F,
    out: &mut Vec<EdgeDto>,
) -> Result<()>
where
    F: FnMut(String, String) -> (String, String, String),
{
    let rs = conn
        .query(cypher)
        .map_err(|e| Error::Other(format!("kuzu query failed: {e}")))?;
    for row in rs {
        let a = match row.first() {
            Some(kuzu::Value::String(s)) => s.clone(),
            _ => continue,
        };
        let b = match row.get(1) {
            Some(kuzu::Value::String(s)) => s.clone(),
            _ => continue,
        };
        let (source, kind, target) = build(a, b);
        out.push(EdgeDto {
            id: edge_id(&source, &kind, &target),
            source,
            target,
            kind,
            props: json!({}),
        });
    }
    Ok(())
}

/// Extract a `String` from `row[idx]`, returning `None` for other variants.
pub fn string(row: &[kuzu::Value], idx: usize) -> Option<String> {
    match row.get(idx) {
        Some(kuzu::Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Extract an `i64` from `row[idx]`, returning `None` for other variants.
pub fn int64(row: &[kuzu::Value], idx: usize) -> Option<i64> {
    match row.get(idx) {
        Some(kuzu::Value::Int64(n)) => Some(*n),
        _ => None,
    }
}
