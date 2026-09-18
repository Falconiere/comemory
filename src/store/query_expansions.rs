//! `query_expansions` row CRUD: the mined (term → expansion) mapping table
//! `store::fts`'s tier-4 lexical ladder reads, rewritten wholesale by
//! `comemory mine --apply` ([`crate::domains::learning::evaluation::mine::apply`]).

use rusqlite::Connection;

use super::{
    orm,
    schema_learning::{QueryExpansions, query_expansions as col},
};
use crate::prelude::*;
use toolu_orm::core::query_column::CommonOps;

/// One mined mapping row to insert via [`insert`].
pub struct NewExpansion<'a> {
    /// Failed query term (tokenized form).
    pub term: &'a str,
    /// Term from the successful rewording.
    pub expansion: &'a str,
    /// Number of (failed, successful) pairs that produced this mapping.
    pub support: i64,
    /// ISO-8601 UTC timestamp of this mine run.
    pub last_mined: &'a str,
}

/// Delete every `query_expansions` row — the first half of `mine --apply`'s
/// replace-all (support is always derived from the current `retrieval_log`,
/// so gc-evicted rows must not linger). Caller commits.
pub fn delete_all(conn: &Connection) -> Result<()> {
    orm::execute(conn, QueryExpansions::delete().to_sql())?;
    Ok(())
}

/// Insert one mined mapping row.
pub fn insert(conn: &Connection, row: &NewExpansion<'_>) -> Result<()> {
    orm::execute(
        conn,
        QueryExpansions::insert()
            .set(&col::term, row.term)
            .set(&col::expansion, row.expansion)
            .set(&col::support, row.support)
            .set(&col::last_mined, row.last_mined)
            .to_sql(),
    )?;
    Ok(())
}

/// One `query_expansions` row matched by [`matching_terms`].
pub struct MatchedExpansion {
    /// The failed query term this row's `expansion` was mined for.
    pub term: String,
    /// The term the mining pass learned to add for it.
    pub expansion: String,
    /// How many reformulation pairs support the mapping.
    pub support: i64,
}

/// Rows whose `term` is one of `terms`, strongest-support first (ties
/// broken on `term`, then `expansion`), capped at `limit` — behind
/// `retrieval::suggest`'s "expansions" list. An empty `terms` slice short-circuits
/// to an empty result rather than building an invalid `IN ()` clause.
pub fn matching_terms(
    conn: &Connection,
    terms: &[String],
    limit: usize,
) -> Result<Vec<MatchedExpansion>> {
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let terms = terms.iter().cloned().map(Into::into).collect::<Vec<_>>();
    orm::query_all(
        conn,
        QueryExpansions::select()
            .columns_typed(&[&col::term, &col::expansion, &col::support])
            .filter(col::term.in_list(&terms))
            .order_by(col::support.desc())
            .order_by(col::term.asc())
            .order_by(col::expansion.asc())
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .to_sql(),
        |r| {
            Ok(MatchedExpansion {
                term: r.get(0)?,
                expansion: r.get(1)?,
                support: r.get(2)?,
            })
        },
    )
}

/// Total `query_expansions` row count — behind `domains::learning::console`'s summary
/// tile and its paged `expansions` list.
pub fn count(conn: &Connection) -> Result<u64> {
    Ok(
        orm::query_one(conn, QueryExpansions::select().to_count_sql(), |r| {
            r.get::<_, i64>(0)
        })? as u64,
    )
}

/// One `query_expansions` row as returned by [`page`].
pub struct MinedRow {
    /// Failed query term (`query_expansions.term`).
    pub term: String,
    /// Term from the successful rewording (`.expansion`).
    pub expansion: String,
    /// Observation count backing the mapping (`.support`).
    pub support: i64,
    /// ISO-8601 UTC timestamp of the mining run that wrote the row.
    pub last_mined: String,
}

/// One page of mined expansions, strongest support first — `term` then
/// `expansion` break the tie, matching `(term, expansion)`'s primary key so
/// a page boundary is stable. `sql_limit` is a raw SQLite `LIMIT` value; a
/// negative value means "no limit" (the caller's `Page`-"all" sentinel).
pub fn page(conn: &Connection, sql_limit: i64, offset: i64) -> Result<Vec<MinedRow>> {
    orm::query_all(
        conn,
        QueryExpansions::select()
            .columns_typed(&[&col::term, &col::expansion, &col::support, &col::last_mined])
            .order_by(col::support.desc())
            .order_by(col::term.asc())
            .order_by(col::expansion.asc())
            .limit(sql_limit)
            .offset(offset)
            .to_sql(),
        |r| {
            Ok(MinedRow {
                term: r.get(0)?,
                expansion: r.get(1)?,
                support: r.get(2)?,
                last_mined: r.get(3)?,
            })
        },
    )
}

#[cfg(test)]
#[path = "tests/query_expansions.rs"]
mod tests;
