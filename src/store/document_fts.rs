//! `document_fts` insert/delete helpers plus the BM25 MATCH query leg —
//! mirrors `store::fts`'s code leg (`index_code`/`search_code`) for the
//! document domain. Per-leg BM25 column weights land in a later step;
//! this module exposes the raw ranked rows over the default
//! (unweighted) `bm25()`.

use rusqlite::{Connection, params_from_iter};

use super::{
    orm,
    schema_documents::{DocumentFts, document_fts as col},
};
use crate::prelude::*;
use crate::store::fts;
use toolu_orm::core::query_column::CommonOps;

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
    orm::execute(
        conn,
        DocumentFts::insert()
            .set(&col::document_id, document_id)
            .set(&col::ordinal, ordinal)
            .set(&col::title, title)
            .set(&col::headings, headings)
            .set(&col::passage, passage)
            .set(&col::path_tokens, path_tokens)
            .to_sql(),
    )?;
    Ok(())
}

/// Delete every `document_fts` row for `document_id`. `document_fts` is
/// a virtual table with no FK, so this is the writer's explicit
/// cleanup counterpart to [`crate::store::documents::delete_document`]
/// (which only cascades the plain `documents`/`document_chunks` rows).
pub fn delete_document(conn: &Connection, document_id: &str) -> Result<()> {
    orm::execute(
        conn,
        DocumentFts::delete()
            .filter(col::document_id.eq(document_id))
            .to_sql(),
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
    let score =
        toolu_orm::core::fts5::bm25(col::document_id.table, &[]).map_err(orm::build_error)?;
    let predicate = toolu_orm::core::expr::Expr::table_match(col::document_id.table, match_expr)
        .map_err(orm::build_error)?;
    let (sql, params) = DocumentFts::select()
        .columns_typed(&[&col::document_id, &col::ordinal])
        .column_expr(&format!("-{}", score.sql()), "score")
        .filter(predicate)
        .order_by(toolu_orm::core::expr::OrderBy::alias_desc("score"))
        .limit(k as i64)
        .to_sql();
    fts::run_fts_query(conn, &sql, params_from_iter(params), |row| {
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
