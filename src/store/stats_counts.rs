//! The corpus counters behind `comemory stats` / `GET /api/v1/stats`: a
//! generic scoped `COUNT(*)`, a table-wide `COUNT(*)`, and the logical
//! database size. Moved out of `maintenance::stats` (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).

use rusqlite::Connection;

use super::{
    orm,
    schema_code::{CodeSymbols, code_symbols},
    schema_documents::{Documents, documents},
    schema_memory::{Memories, memories},
};
use crate::prelude::*;
use toolu_orm::core::query_column::CommonOps;
use toolu_orm::query::select::SelectBuilder;

/// A repo-scoped corpus counter, including the memory visibility policy.
#[derive(Clone, Copy)]
pub enum Corpus {
    /// Memories that have not been soft-deleted.
    LiveMemories,
    /// Soft-deleted memories awaiting garbage collection.
    TrashedMemories,
    /// Every indexed code symbol, including chunks.
    CodeSymbols,
    /// Every indexed document.
    Documents,
}

/// Count one corpus, optionally narrowed to a repo. The corpus chooses its
/// table and visibility predicate; callers never supply SQL fragments.
pub fn scoped_count(conn: &Connection, corpus: Corpus, repo: Option<&str>) -> Result<u64> {
    let (mut query, repo_column) = match corpus {
        Corpus::LiveMemories => (
            Memories::select().filter(memories::deleted_at.is_null()),
            memories::repo,
        ),
        Corpus::TrashedMemories => (
            Memories::select().filter(memories::deleted_at.is_not_null()),
            memories::repo,
        ),
        Corpus::CodeSymbols => (CodeSymbols::select(), code_symbols::repo),
        Corpus::Documents => (Documents::select(), documents::repo),
    };
    if let Some(repo) = repo {
        query = query.filter(repo_column.eq(repo));
    }
    Ok(orm::query_one(conn, query.to_count_sql(), |r| r.get::<_, i64>(0))? as u64)
}

/// `COUNT(*)` over the whole of a statically named table, with its identifier
/// quoted by the ORM builder.
pub fn count_table(conn: &Connection, table: &'static str) -> Result<u64> {
    let query = SelectBuilder::new(table);
    Ok(orm::query_one(conn, query.to_count_sql(), |r| r.get::<_, i64>(0))? as u64)
}

/// `page_count * page_size` — the logical size of the database (not the
/// file's length on disk: WAL-mode pages not yet checkpointed live in
/// `comemory.db-wal` and the two legitimately disagree).
pub fn db_bytes(conn: &Connection) -> Result<u64> {
    let pages: i64 = conn.pragma_query_value(None, "page_count", |r| r.get(0))?;
    let size: i64 = conn.pragma_query_value(None, "page_size", |r| r.get(0))?;
    Ok((pages.max(0) as u64).saturating_mul(size.max(0) as u64))
}

#[cfg(test)]
#[path = "tests/stats_counts.rs"]
mod tests;
