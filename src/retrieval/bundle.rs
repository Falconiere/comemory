//! Build the JSON shape emitted by `comemory context`.
//!
//! `assemble` joins the SQLite `memories` table with `code_symbols`
//! through the `edges` graph table so a single bundle contains the
//! matched memories, the code symbols they reference, and a flat list
//! of relation triples.

use rusqlite::Connection;
use serde::Serialize;

use crate::graph::edges;
use crate::prelude::*;

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

/// Assemble a [`Bundle`] for `query`, expanding each memory id by one
/// hop along `references_symbol` edges so the JSON ships the referenced
/// code snippets alongside the matched memories.
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

        for (dst_kind, dst_id) in edges::outgoing(conn, "memory", id, "references_symbol")? {
            relations.push(RelationRow {
                from: format!("memory:{id}"),
                rel: "references_symbol".into(),
                to: format!("{dst_kind}:{dst_id}"),
            });
            if let Some((repo, path, symbol, snippet)) = code_ref_lookup(conn, &dst_id)? {
                code_refs.push(CodeRow {
                    repo,
                    path,
                    symbol,
                    snippet,
                });
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

/// Look up the `code_symbols` row addressed by a `<repo>:<path>:<symbol>`
/// edge destination. Returns `Ok(None)` when the address shape is wrong
/// or no row matches so the caller can skip silently.
fn code_ref_lookup(
    conn: &Connection,
    dst_id: &str,
) -> Result<Option<(String, String, String, String)>> {
    let parts: Vec<&str> = dst_id.splitn(3, ':').collect();
    if parts.len() != 3 {
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
