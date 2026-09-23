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
use crate::store::remote_document_view;
use toolu_orm::core::query_column::CommonOps;

/// Which index a hit came from, and the key that identifies it there.
///
/// The two sides cannot share one id: a local document is keyed by the hash of
/// a path on THIS machine, and a pulled revision by the hash of a repository
/// and a repository-relative path. Carrying the distinction in the hit is what
/// lets the reader fetch the right rows and say which side answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitSource {
    /// A locally indexed document, by its `documents.id`.
    Local(String),
    /// A pulled revision, by canonical repository and shared id.
    Shared {
        /// Canonical repository.
        repo: String,
        /// 32 lowercase hex chars over `repo` and the document's path.
        shared_id: String,
    },
}

/// One document MATCH hit: where its chunk lives, the ordinal within that
/// document, and a higher-is-better relevance score (negated `bm25()`,
/// matching the `RoutedHit` lexical convention `store::edge_fts::EdgeFtsHit`
/// also follows).
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentFtsHit {
    /// Which index matched, and its key there.
    pub source: HitSource,
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

/// The local half: every matching passage of a document this machine indexed.
const LOCAL_PASSAGES: &str = "SELECT \
         '' AS repo, \
         document_fts.document_id AS key, \
         CAST(document_fts.ordinal AS INTEGER) AS ordinal, \
         -bm25(document_fts) AS score \
       FROM document_fts \
      WHERE document_fts MATCH ?1";

/// Run a BM25 search over BOTH document indexes and return the top-`k` chunk
/// hits, best first. FTS5 MATCH parse errors are downgraded to an empty
/// result, same as [`fts::search_code`].
///
/// The union sits INSIDE the limit, so `k` bounds the corpus rather than each
/// half: a query where the pulled side scores higher throughout still gets `k`
/// hits, and one where the local side does is unaffected by how much a peer
/// shared. Putting the limit above the union — one `k` per side, trimmed after
/// — would make the ranking depend on which machines had synced.
///
/// Hand-written because the builder has no compound-select form; the shared
/// half is [`remote_document_view::SHARED_PASSAGES`] verbatim, so the
/// precedence and approval rules have one definition.
pub fn search(conn: &Connection, query: &str, k: usize) -> Result<Vec<DocumentFtsHit>> {
    let match_expr = fts::build_match_query(query);
    if match_expr.is_empty() || k == 0 {
        return Ok(Vec::new());
    }
    let shared = remote_document_view::SHARED_PASSAGES;
    let sql = format!(
        "SELECT repo, key, ordinal, score FROM ({LOCAL_PASSAGES}) \
         UNION ALL \
         SELECT repo, shared_id AS key, ordinal, score FROM ({shared}) \
          ORDER BY score DESC, repo ASC, key ASC, ordinal ASC \
          LIMIT ?2"
    );
    fts::run_fts_query(
        conn,
        &sql,
        params_from_iter(vec![
            rusqlite::types::Value::from(match_expr),
            rusqlite::types::Value::from(i64::try_from(k).unwrap_or(i64::MAX)),
        ]),
        |row| {
            let repo: String = row.get(0)?;
            let key: String = row.get(1)?;
            let source = if repo.is_empty() {
                HitSource::Local(key)
            } else {
                HitSource::Shared {
                    repo,
                    shared_id: key,
                }
            };
            Ok(DocumentFtsHit {
                source,
                ordinal: row.get(2)?,
                score: row.get(3)?,
            })
        },
    )
}

#[cfg(test)]
#[path = "tests/document_fts.rs"]
mod tests;
