//! `query_expansions` row CRUD: the mined (term → expansion) mapping table
//! `store::fts`'s tier-4 lexical ladder reads, rewritten wholesale by
//! `comemory mine --apply` ([`crate::eval::mine::apply`]).

use rusqlite::{Connection, ToSql, params, params_from_iter};

use crate::prelude::*;

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
    conn.execute("DELETE FROM query_expansions", [])?;
    Ok(())
}

/// Insert one mined mapping row.
pub fn insert(conn: &Connection, row: &NewExpansion<'_>) -> Result<()> {
    conn.execute(
        "INSERT INTO query_expansions(term, expansion, support, last_mined)
         VALUES (?1, ?2, ?3, ?4)",
        params![row.term, row.expansion, row.support, row.last_mined],
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
/// `api::suggest`'s "expansions" list. An empty `terms` slice short-circuits
/// to an empty result rather than building an invalid `IN ()` clause.
pub fn matching_terms(
    conn: &Connection,
    terms: &[String],
    limit: usize,
) -> Result<Vec<MatchedExpansion>> {
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (1..=terms.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT term, expansion, support FROM query_expansions \
          WHERE term IN ({placeholders}) \
          ORDER BY support DESC, term ASC, expansion ASC LIMIT ?{}",
        terms.len() + 1
    );
    let mut binds: Vec<Box<dyn ToSql>> = terms
        .iter()
        .map(|t| Box::new(t.clone()) as Box<dyn ToSql>)
        .collect();
    // A `limit` above i64::MAX cannot describe a reachable row count, so
    // saturating is the only meaningful conversion; SQLite treats any such
    // value as "no limit" regardless. Preserved from the api::suggest call
    // site this moved from.
    binds.push(Box::new(i64::try_from(limit).unwrap_or(i64::MAX)));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params_from_iter(binds.iter().map(std::convert::AsRef::as_ref)),
        |r| {
            Ok(MatchedExpansion {
                term: r.get(0)?,
                expansion: r.get(1)?,
                support: r.get(2)?,
            })
        },
    )?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Error::from)
}

#[cfg(test)]
#[path = "tests/query_expansions.rs"]
mod tests;
