//! Build the JSON shape emitted by `comemory context`.
//!
//! `assemble` joins the SQLite `memories` table with `code_symbols`
//! through the `edges` graph table so a single bundle contains the
//! matched memories, the code symbols they reference via any of the four
//! context rels (`references_file`, `references_symbol`, `relates_to`,
//! `supersedes`) walked to depth ≤ 2, and a flat list of relation triples.

use rusqlite::Connection;
use serde::Serialize;

use crate::prelude::*;

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
    /// Code-symbol rows reached by walking `references_symbol` edges.
    pub code_refs: Vec<CodeRow>,
    /// Flat list of relation triples for downstream UIs.
    pub relations: Vec<RelationRow>,
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
    /// Repo identifier the symbol lives in.
    pub repo: String,
    /// Repo-relative path of the file.
    pub path: String,
    /// Qualified symbol name.
    pub symbol: String,
    /// Source snippet for the symbol.
    pub snippet: String,
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
/// every `references_symbol` destination that resolves in `code_symbols`.
pub fn assemble<'a>(
    conn: &Connection,
    query: &'a str,
    memory_ids: &[String],
) -> Result<Bundle<'a>> {
    let mut memories = Vec::new();
    let mut relations = Vec::new();
    let mut code_refs = Vec::new();

    for id in memory_ids {
        let (kind, body): (String, String) =
            conn.query_row("SELECT kind, body FROM memories WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        memories.push(MemoryBundleRow {
            id: id.clone(),
            kind,
            body,
            score: 0.0,
        });

        // Walk all four context rels at depth ≤ 2 from this memory node.
        let walked = walk_context_edges(conn, id, 2)?;
        for e in walked {
            relations.push(RelationRow {
                from: format!("{}:{}", e.src_kind, e.src_id),
                rel: e.rel.clone(),
                to: format!("{}:{}", e.dst_kind, e.dst_id),
            });
            if e.rel == "references_symbol" {
                if let Some((repo, path, symbol, snippet)) = code_ref_lookup(conn, &e.dst_id)? {
                    code_refs.push(CodeRow {
                        repo,
                        path,
                        symbol,
                        snippet,
                    });
                }
            }
        }
    }
    Ok(Bundle {
        query,
        memories,
        code_refs,
        relations,
    })
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
         SELECT src_kind, src_id, dst_kind, dst_id, rel FROM walk",
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

/// Look up the `code_symbols` row addressed by a `<repo>:<path>:<symbol>`
/// edge destination. Returns `Ok(None)` when the address shape is wrong
/// or no row matches so the caller can skip silently.
fn code_ref_lookup(
    conn: &Connection,
    dst_id: &str,
) -> Result<Option<(String, String, String, String)>> {
    let parts: Vec<&str> = dst_id.splitn(3, ':').collect();
    if parts.len() != 3 {
        tracing::warn!(
            dst_id = %dst_id,
            "malformed references_symbol edge destination (expected <repo>:<path>:<symbol>); skipping"
        );
        return Ok(None);
    }
    let (repo, path, symbol) = (parts[0], parts[1], parts[2]);
    let row = conn
        .query_row(
            "SELECT snippet FROM code_symbols \
              WHERE repo = ?1 AND path = ?2 AND symbol = ?3 LIMIT 1",
            rusqlite::params![repo, path, symbol],
            |r| r.get::<_, String>(0),
        )
        .ok();
    Ok(row.map(|s| (repo.into(), path.into(), symbol.into(), s)))
}
