//! Build the JSON shape emitted by `comemory context`.
//!
//! `assemble` joins the SQLite `memories` table with `code_symbols`
//! through the `edges` graph table so a single bundle contains the
//! matched memories, the code symbols they reference via any of the four
//! context rels (`references_file`, `references_symbol`, `relates_to`,
//! `supersedes`) walked to depth ≤ 2, and a flat list of relation triples.
//!
//! Code refs are ranked by the four-prior product from
//! [`crate::retrieval::code_prior`] (PageRank, activation, working-set
//! affinity, feedback). There is no relevance term: refs are
//! address-resolved by the graph walk — every one is "fully relevant" to
//! the memory that cites it — so only the priors can order them.

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::code_prior::{self, CodePriorParts, Signals};
use crate::retrieval::code_ref_collect::{self, RawRef};
use crate::retrieval::code_ref_fetch::RefStatusCache;
use crate::retrieval::code_rerank::WorkingSet;

/// One row returned by [`walk_context_edges`]: a directed edge from the graph.
struct ContextEdge {
    src_kind: String,
    src_id: String,
    dst_kind: String,
    dst_id: String,
    rel: String,
}

/// JSON-serializable retrieval bundle returned to `comemory context`.
#[derive(Serialize)]
pub struct Bundle<'a> {
    /// Original query string.
    pub query: &'a str,
    /// Memory rows surfaced by the router.
    pub memories: Vec<MemoryBundleRow>,
    /// Code-symbol rows reached by walking `references_symbol` edges,
    /// prior-ranked (see the module doc): resolved refs first by
    /// descending `rank_parts.final_score`, then unresolved refs, each
    /// group tie-broken by `(path, symbol)`.
    pub code_refs: Vec<CodeRow>,
    /// Flat list of relation triples for downstream UIs.
    pub relations: Vec<RelationRow>,
    /// `code_symbols` rowids of the code refs that resolved to an indexed
    /// row — the access-trackable identities `comemory context` bumps
    /// under its tracking flag (the code-side twin of the memory access
    /// bump). Not part of the JSON contract: skipped from serialization
    /// so the `#[serde(flatten)]` envelope shape stays unchanged.
    #[serde(skip)]
    pub resolved_code_ids: Vec<i64>,
}

/// One memory row inside a [`Bundle`].
#[derive(Serialize)]
pub struct MemoryBundleRow {
    /// Memory id (8-hex prefix of `sha256(body.trim_end())`).
    pub id: String,
    /// Memory kind (decision|bug|convention|discovery|pattern|note).
    pub kind: String,
    /// Full memory body.
    pub body: String,
    /// Caller-supplied score (defaults to `0.0` when assembling).
    pub score: f32,
}

/// One code-symbol row inside a [`Bundle`].
#[derive(Serialize)]
pub struct CodeRow {
    /// Qualified address `<repo>:<path>[:<symbol>]` of the reference.
    pub id: String,
    /// Repo identifier the symbol lives in.
    pub repo: String,
    /// Repo-relative path of the file.
    pub path: String,
    /// Qualified symbol name; empty for a file ref.
    pub symbol: String,
    /// Source snippet for the symbol; empty when the ref did not resolve
    /// to an indexed `code_symbols` row.
    pub snippet: String,
    /// First source line of the symbol; `None` for file refs or unresolved
    /// symbols (no current index covers them).
    pub line: Option<i64>,
    /// First line of the symbol's snippet (its signature); `None` when the
    /// snippet is empty.
    pub signature: Option<String>,
    /// Freshness verdict (`fresh|stale|ghost|unpinned|unknown`).
    pub status: String,
    /// Ranking score — the code-prior product when resolved, else `0.0`.
    pub score: f64,
    /// Four-prior breakdown behind this ref's position. `None` (and
    /// omitted from JSON) when the ref did not resolve to a
    /// `code_symbols` row — a memory may cite symbols before
    /// `comemory index-code` has indexed them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_parts: Option<CodePriorParts>,
}

/// One relation triple inside a [`Bundle`].
#[derive(Serialize)]
pub struct RelationRow {
    /// `<src_kind>:<src_id>` address of the source node.
    pub from: String,
    /// Relation label.
    pub rel: String,
    /// `<dst_kind>:<dst_id>` address of the destination node.
    pub to: String,
}

/// Assemble a [`Bundle`] for `query`, expanding each memory id by walking
/// `references_file`, `references_symbol`, `relates_to`, and `supersedes`
/// edges up to depth 2 via a recursive CTE. Code snippets are pulled for
/// every `references_symbol` destination that resolves in `code_symbols`,
/// and the resulting refs are prior-ranked against `working_set` (see
/// [`rank_code_refs`]).
pub fn assemble<'a>(
    conn: &Connection,
    cfg: &Config,
    query: &'a str,
    memory_ids: &[String],
    working_set: &WorkingSet,
) -> Result<Bundle<'a>> {
    let mut memories = Vec::new();
    let mut relations = Vec::new();
    let mut raw_refs = Vec::new();

    for id in memory_ids {
        collect_memory(conn, id, &mut memories, &mut relations, &mut raw_refs)?;
    }
    // Snapshot the resolved ids before `rank_code_refs` consumes the raw
    // refs — these are the rows `context` self-reinforces under tracking.
    let resolved_code_ids: Vec<i64> = raw_refs.iter().filter_map(|r| r.symbol_id).collect();
    let code_refs = rank_code_refs(conn, cfg, raw_refs, working_set)?;
    Ok(Bundle {
        query,
        memories,
        code_refs,
        relations,
        resolved_code_ids,
    })
}

/// Load one memory row and append its bundle row, walked relations, and any
/// resolved code refs into the caller's accumulators.
///
/// Tolerates a missing/soft-deleted memory row: the router emits ids drawn
/// from independent indices (FTS5, vec0) that may drift past a soft-delete or
/// rebuild, and a stale id should skip cleanly rather than abort the bundle.
fn collect_memory(
    conn: &Connection,
    id: &str,
    memories: &mut Vec<MemoryBundleRow>,
    relations: &mut Vec<RelationRow>,
    raw_refs: &mut Vec<RawRef>,
) -> Result<()> {
    let row = conn
        .query_row(
            "SELECT kind, body FROM memories \
              WHERE id = ?1 AND deleted_at IS NULL",
            [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .ok();
    let Some((kind, body)) = row else {
        return Ok(());
    };
    memories.push(MemoryBundleRow {
        id: id.to_string(),
        kind,
        body,
        score: 0.0,
    });

    // Pinned anchors for this memory, keyed by `(rel, dst_id)`, so each walked
    // reference edge can carry the blob captured at save.
    let anchors = code_ref_collect::anchor_map(conn, id)?;
    // Walk all four context rels at depth ≤ 2 from this memory node.
    let walked = walk_context_edges(conn, id, 2)?;
    for e in walked {
        relations.push(RelationRow {
            from: format!("{}:{}", e.src_kind, e.src_id),
            rel: e.rel.clone(),
            to: format!("{}:{}", e.dst_kind, e.dst_id),
        });
        if let Some(raw) = code_ref_collect::ref_from_edge(conn, &e.rel, &e.dst_id, &anchors)? {
            raw_refs.push(raw);
        }
    }
    Ok(())
}

/// Score every resolved ref with the four-prior product (no relevance
/// term — see the module doc) and sort: resolved refs by descending
/// `final_score`, ties on `(path, symbol)`; unresolved refs after them,
/// also `(path, symbol)`-ordered. Follows the same pooled discipline as
/// `rerank_code`: each ref's [`code_prior::signals`] row is fetched once,
/// the rank-prior median is derived from those rows via
/// [`code_prior::median_file_rank`], and the whole pool is scored under
/// one shared clock and one shared affinity cache.
fn rank_code_refs(
    conn: &Connection,
    cfg: &Config,
    raw_refs: Vec<RawRef>,
    working_set: &WorkingSet,
) -> Result<Vec<CodeRow>> {
    // Fetch every resolved ref's signals in ONE batched query, then map back
    // per-ref preserving order. A row that vanished between lookup and scoring
    // (raced re-index delete) has no map entry, degrading to the same `None`
    // the per-ref `signals` returned — it sorts with the unresolved refs.
    let ids: Vec<i64> = raw_refs.iter().filter_map(|r| r.symbol_id).collect();
    let by_id = code_prior::signals_batch(conn, &ids)?;
    let sigs: Vec<Option<Signals>> = raw_refs
        .iter()
        // `.cloned()` (not `remove`) so two refs sharing a `symbol_id` each
        // get the row, exactly as the per-ref `signals` fetch did.
        .map(|r| r.symbol_id.and_then(|id| by_id.get(&id).cloned()))
        .collect();
    let median = code_prior::median_file_rank(
        sigs.iter()
            .flatten()
            .map(|s| ((s.repo.as_str(), s.path.as_str()), s.rank_score)),
    );
    let mut out = score_refs(conn, cfg, raw_refs, sigs, working_set, median)?;
    out.sort_by(|a, b| match (&a.rank_parts, &b.rank_parts) {
        (Some(x), Some(y)) => y
            .final_score
            .total_cmp(&x.final_score)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.symbol.cmp(&b.symbol)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.path.cmp(&b.path).then_with(|| a.symbol.cmp(&b.symbol)),
    });
    Ok(out)
}

/// Score each ref against its (possibly `None`) signals row, building the
/// unsorted [`CodeRow`] pool under one shared clock and affinity cache.
fn score_refs(
    conn: &Connection,
    cfg: &Config,
    raw_refs: Vec<RawRef>,
    sigs: Vec<Option<Signals>>,
    working_set: &WorkingSet,
    median: f64,
) -> Result<Vec<CodeRow>> {
    let now = OffsetDateTime::now_utc();
    let mut affinity_cache: BTreeMap<String, f64> = BTreeMap::new();
    let mut status_cache = RefStatusCache::default();
    let mut out = Vec::with_capacity(raw_refs.len());
    for (r, sig) in raw_refs.into_iter().zip(sigs) {
        let rank_parts = match sig {
            Some(sig) => Some(code_prior::priors(
                conn,
                cfg,
                now,
                &sig,
                working_set,
                median,
                &mut affinity_cache,
            )?),
            None => None,
        };
        out.push(build_code_row(conn, &mut status_cache, r, rank_parts));
    }
    Ok(out)
}

/// Assemble one [`CodeRow`]: classify freshness against the live repo state and
/// derive the nav-minimal display fields (`line`, `signature`, `score`).
fn build_code_row(
    conn: &Connection,
    status_cache: &mut RefStatusCache,
    r: RawRef,
    rank_parts: Option<CodePriorParts>,
) -> CodeRow {
    let status = status_cache.status(
        conn,
        &r.repo,
        &r.path,
        r.is_symbol,
        r.pinned_blob.as_deref(),
        r.symbol_id.is_some(),
    );
    let signature = r
        .snippet
        .lines()
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let score = rank_parts.as_ref().map(|p| p.final_score).unwrap_or(0.0);
    CodeRow {
        id: r.id,
        repo: r.repo,
        path: r.path,
        symbol: r.symbol,
        snippet: r.snippet,
        line: r.line_start,
        signature,
        status: status.as_str().to_string(),
        score,
        rank_parts,
    }
}

/// Walk `references_file`, `references_symbol`, `relates_to`, and `supersedes`
/// edges starting from `(memory, start_id)` up to `max_depth` hops using a
/// recursive CTE. Returns one [`ContextEdge`] per traversed edge.
fn walk_context_edges(
    conn: &Connection,
    start_id: &str,
    max_depth: u32,
) -> Result<Vec<ContextEdge>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE walk(src_kind, src_id, dst_kind, dst_id, rel, depth) AS (
             SELECT e.src_kind, e.src_id, e.dst_kind, e.dst_id, e.rel, 1
               FROM edges e
              WHERE e.src_kind = 'memory' AND e.src_id = ?1
                AND e.rel IN ('references_file','references_symbol','relates_to','supersedes')
             UNION
             SELECT e.src_kind, e.src_id, e.dst_kind, e.dst_id, e.rel, w.depth + 1
               FROM edges e
               JOIN walk w ON e.src_kind = w.dst_kind AND e.src_id = w.dst_id
              WHERE e.rel IN ('references_file','references_symbol','relates_to','supersedes')
                AND w.depth < ?2
         )
         SELECT DISTINCT src_kind, src_id, dst_kind, dst_id, rel \
           FROM walk \
          ORDER BY rel, src_kind, src_id, dst_kind, dst_id",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![start_id, max_depth as i64], |r| {
            Ok(ContextEdge {
                src_kind: r.get(0)?,
                src_id: r.get(1)?,
                dst_kind: r.get(2)?,
                dst_id: r.get(3)?,
                rel: r.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}
