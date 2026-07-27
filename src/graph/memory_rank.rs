//! Memory-graph PageRank: the save/rebuild/delete post-pass that scores
//! every live memory by structural centrality and projects the result onto
//! `memories.rank_score`, mirroring what [`crate::graph::materialize`] does
//! for the code side.
//!
//! The graph is derived at compute time, never persisted: hub rels
//! (`in_repo`, `authored_by`, `tagged`) are excluded because they connect
//! the whole corpus, and two memories citing the same file or symbol are
//! joined by an in-memory undirected co-citation edge. Persisting that
//! derived edge would need a new `edges.rel` kind — a CHECK rebuild
//! migration — to buy nothing: the self-join is cheap and runs inside the
//! transaction that writes the scores.

use std::collections::BTreeMap;

use rusqlite::{Connection, Transaction, params};

use crate::graph::pagerank;
use crate::prelude::*;

/// A derived memory graph: sorted live memory ids (the dense node index,
/// by position) paired with weighted `(src, dst, weight)` edges over those
/// indices — the exact shape [`pagerank::pagerank`] consumes.
pub type MemoryGraph = (Vec<String>, Vec<(u32, u32, f64)>);

/// Direct memory→memory relations, read in the direction they are stored
/// (src = the newer/building memory) so PageRank mass flows to the
/// referenced memory. The `ORDER BY` pins f64 accumulation order to the
/// logical graph rather than rowid order — the `project_pagerank` rule.
const DIRECT_EDGES_SQL: &str = "SELECT src_id, dst_id, weight FROM edges \
      WHERE src_kind = 'memory' AND dst_kind = 'memory' \
        AND rel IN ('supersedes','conflicts_with','derived_from','relates_to') \
     ORDER BY rel, src_id, dst_id";

/// Co-citation: one row per unordered memory pair that references the same
/// target through the same rel, weighted by the number of shared targets.
/// `a.src_id < b.src_id` drops self-pairs and yields each pair once; the
/// per-rel join sidesteps the bare-vs-`file:`-prefixed dst id divergence,
/// since ids from different rels never meet.
const CO_CITATION_SQL: &str = "SELECT a.src_id, b.src_id, CAST(COUNT(*) AS REAL) AS w \
       FROM edges a \
       JOIN edges b ON a.rel = b.rel AND a.dst_kind = b.dst_kind \
                   AND a.dst_id = b.dst_id AND a.src_id < b.src_id \
      WHERE a.src_kind = 'memory' AND b.src_kind = 'memory' \
        AND a.rel IN ('references_file','references_symbol','co_activated') \
      GROUP BY a.src_id, b.src_id \
      ORDER BY a.src_id, b.src_id";

/// Recompute PageRank over the live-memory graph and write every
/// `memories.rank_score` in one transaction. A corpus with no live
/// memories is a no-op. Callers treat failure as best-effort — see
/// [`refresh_best_effort`].
pub fn materialize_memory_rank(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    let (nodes, graph) = derive_memory_graph(&tx)?;
    if nodes.is_empty() {
        return Ok(());
    }
    let scores = pagerank::pagerank(nodes.len(), &graph);
    write_scores(&tx, &nodes, &scores)?;
    tx.commit()?;
    Ok(())
}

/// Best-effort [`materialize_memory_rank`]: warn and continue on any
/// error. Every trigger (save, rebuild, soft-delete) calls this AFTER its
/// own transaction has committed, so a failed refresh can only cost rank
/// freshness, never the primary write.
pub fn refresh_best_effort(conn: &mut Connection) {
    if let Err(e) = materialize_memory_rank(conn) {
        tracing::warn!(error = %e, "memory_rank: refresh failed; rank_score left stale");
    }
}

/// Derive the [`MemoryGraph`] from the live memories and their edges.
///
/// `pub` so the flat-mirror test can assert the derived topology directly,
/// without reading it back through the scores.
pub fn derive_memory_graph(conn: &Connection) -> Result<MemoryGraph> {
    let nodes = live_memory_ids(conn)?;
    let index: BTreeMap<&str, u32> = nodes
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i as u32))
        .collect();
    let mut graph: Vec<(u32, u32, f64)> = Vec::new();
    push_direct_edges(conn, &index, &mut graph)?;
    push_co_citation_edges(conn, &index, &mut graph)?;
    Ok((nodes, graph))
}

/// Every live memory id, sorted ascending — the deterministic dense-index
/// mapping PageRank needs. Soft-deleted rows leave the node universe, so
/// their mass redistributes on the next recompute.
fn live_memory_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM memories WHERE deleted_at IS NULL ORDER BY id")?;
    let rows = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(rows)
}

/// Fetch `(src_id, dst_id, weight)` triples for one edge query, with the
/// weight already widened to f64 so both edge sources share one row shape.
fn fetch_edges(conn: &Connection, sql: &str) -> Result<Vec<(String, String, f64)>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Append the directed memory→memory relation edges to `graph`.
fn push_direct_edges(
    conn: &Connection,
    index: &BTreeMap<&str, u32>,
    graph: &mut Vec<(u32, u32, f64)>,
) -> Result<()> {
    for (src, dst, weight) in fetch_edges(conn, DIRECT_EDGES_SQL)? {
        if let Some((s, d)) = resolve(index, &src, &dst) {
            graph.push((s, d, weight));
        }
    }
    Ok(())
}

/// Append the derived co-citation edges to `graph`, each expanded to both
/// directions — the undirected in-memory expansion `co_changed` uses on the
/// code side, so neither co-citing memory is the donor.
fn push_co_citation_edges(
    conn: &Connection,
    index: &BTreeMap<&str, u32>,
    graph: &mut Vec<(u32, u32, f64)>,
) -> Result<()> {
    for (src, dst, weight) in fetch_edges(conn, CO_CITATION_SQL)? {
        if let Some((s, d)) = resolve(index, &src, &dst) {
            graph.push((s, d, weight));
            graph.push((d, s, weight));
        }
    }
    Ok(())
}

/// Dense indices for an edge's endpoints, or `None` when either id is not a
/// live memory — a dangling relation target (frontmatter may name an id
/// that was never saved) or a soft-deleted endpoint.
fn resolve(index: &BTreeMap<&str, u32>, src: &str, dst: &str) -> Option<(u32, u32)> {
    match (index.get(src), index.get(dst)) {
        (Some(&s), Some(&d)) => Some((s, d)),
        _ => {
            tracing::debug!(
                src,
                dst,
                "memory_rank: endpoint is not a live memory; skipping edge"
            );
            None
        }
    }
}

/// Write one score per node, positionally aligned with `nodes` (both come
/// from the same dense index). Split out of [`materialize_memory_rank`] so
/// the prepared statement's borrow of `tx` ends before the commit.
fn write_scores(tx: &Transaction<'_>, nodes: &[String], scores: &[f64]) -> Result<()> {
    let mut update = tx.prepare("UPDATE memories SET rank_score = ?1 WHERE id = ?2")?;
    for (id, score) in nodes.iter().zip(scores) {
        update.execute(params![score, id])?;
    }
    Ok(())
}
