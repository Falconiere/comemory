//! `Graph::seed_memory_layer` and `Graph::seed_all` — full-graph bootstrap
//! queries used by the graph viewer.

use serde_json::json;

use crate::graph::query::{int64, push_edges, push_edges_memory_layer, push_nodes, string};
use crate::graph::upsert::Graph;
use crate::prelude::*;
use crate::serve::dto::{GraphPayload, NodeDto};

impl Graph {
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
}
