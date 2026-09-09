//! `query_expansions` row CRUD: the mined (term → expansion) mapping table
//! `store::fts`'s tier-4 lexical ladder reads, rewritten wholesale by
//! `comemory mine --apply` ([`crate::eval::mine::apply`]).

use rusqlite::{Connection, params};

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

#[cfg(test)]
#[path = "tests/query_expansions.rs"]
mod tests;
