//! Read-only graph queries used by the retrieval pipeline.
//!
//! Submodules each contribute an `impl Graph` block; Rust allows multiple
//! `impl` blocks across files within the same crate.

pub mod detail;
pub mod expand;
pub mod search;
pub mod seed;
pub mod walk;

use kuzu::Value;
use serde_json::json;

use crate::prelude::*;
use crate::serve::dto::{edge_id, EdgeDto, NodeDto};

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
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Extract an `i64` from `row[idx]`, returning `None` for other variants.
pub fn int64(row: &[kuzu::Value], idx: usize) -> Option<i64> {
    match row.get(idx) {
        Some(Value::Int64(n)) => Some(*n),
        _ => None,
    }
}
