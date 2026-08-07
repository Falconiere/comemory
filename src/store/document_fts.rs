//! `document_fts` insert/delete helpers plus the BM25 MATCH query leg —
//! mirrors `store::fts`'s code leg (`index_code`/`search_code`) for the
//! document domain. Per-leg BM25 column weights land in a later step;
//! this module exposes the raw ranked rows over the default
//! (unweighted) `bm25()`.

use rusqlite::{Connection, params};

use crate::prelude::*;
use crate::store::fts;

/// One `document_fts` MATCH hit: the chunk's owning document, its
/// ordinal within that document, and a higher-is-better relevance
/// score (negated `bm25()`, matching the `RoutedHit` lexical
/// convention `store::edge_fts::EdgeFtsHit` also follows).
pub struct DocumentFtsHit {
    /// Owning `documents.id`.
    pub document_id: String,
    /// 0-based chunk position within the document.
    pub ordinal: i64,
    /// Higher-is-better relevance score.
    pub score: f32,
}

/// Insert one `document_fts` row for a chunk.
///
/// `path_tokens` should be the file's raw relative path — like
/// `code_fts.path_tokens` (see [`fts::index_code`]), the `identifier`
/// tokenizer splits `/`/`.`/`-`/camelCase boundaries itself, so
/// pre-lowercasing would destroy them.
pub fn insert(
    conn: &Connection,
    document_id: &str,
    ordinal: i64,
    title: &str,
    headings: &str,
    passage: &str,
    path_tokens: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO document_fts(document_id, ordinal, title, headings, passage, path_tokens) \
         VALUES(?1,?2,?3,?4,?5,?6)",
        params![document_id, ordinal, title, headings, passage, path_tokens],
    )?;
    Ok(())
}

/// Delete every `document_fts` row for `document_id`. `document_fts` is
/// a virtual table with no FK, so this is the writer's explicit
/// cleanup counterpart to [`crate::store::documents::delete_document`]
/// (which only cascades the plain `documents`/`document_chunks` rows).
pub fn delete_document(conn: &Connection, document_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM document_fts WHERE document_id = ?1",
        params![document_id],
    )?;
    Ok(())
}

/// Run a BM25 search over `document_fts` and return the top-`k` chunk
/// hits, best first. FTS5 MATCH parse errors are downgraded to an
/// empty result, same as [`fts::search_code`].
pub fn search(conn: &Connection, query: &str, k: usize) -> Result<Vec<DocumentFtsHit>> {
    let match_expr = fts::build_match_query(query);
    if match_expr.is_empty() || k == 0 {
        return Ok(Vec::new());
    }
    let sql = "SELECT document_id, ordinal, -bm25(document_fts) AS score \
                 FROM document_fts \
                WHERE document_fts MATCH ?1 \
                ORDER BY score DESC \
                LIMIT ?2";
    fts::run_fts_query(conn, sql, params![match_expr, k as i64], |row| {
        Ok(DocumentFtsHit {
            document_id: row.get(0)?,
            ordinal: row.get(1)?,
            score: row.get(2)?,
        })
    })
}

#[cfg(test)]
#[path = "tests/document_fts.rs"]
mod tests;
