//! Bulk `(id, simhash)` scan over live memories.
//!
//! The near-dup surfaces all need the same thing: every live memory's stored
//! fingerprint, optionally narrowed to one repo, optionally minus one row.
//! `save` wants it to find the single closest neighbor of the body being
//! written; `consolidate` wants it to cluster the whole corpus. One query
//! serves both (Binding Rule 1) instead of each growing its own copy.

use rusqlite::Connection;

use crate::prelude::*;

/// One live memory's stored SimHash fingerprint.
///
/// `simhash` is the raw `INTEGER` column: the write path stores
/// `simhash::of_body(body) as i64`, so a reader casts back with `as u64`
/// before handing it to [`crate::simhash::hamming64`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimhashRow {
    /// 8-hex memory id.
    pub id: String,
    /// Stored fingerprint, still in its `i64` on-disk form.
    pub simhash: i64,
}

/// Every live memory's `(id, simhash)`, id-ordered so index-addressed
/// callers (union-find parent vectors) get a stable layout across runs.
/// `repo` narrows the scan; `exclude` drops one id — `save`'s self-exclusion.
///
/// Rows still at `simhash = 0` (the `0004` default, backfilled by
/// `migrate::recompute_simhashes`) come back as-is: only the caller knows
/// whether an absent fingerprint matters.
pub fn live_simhashes(
    conn: &Connection,
    repo: Option<&str>,
    exclude: Option<&str>,
) -> Result<Vec<SimhashRow>> {
    let mut sql = String::from("SELECT id, simhash FROM memories WHERE deleted_at IS NULL");
    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::new();
    if let Some(r) = repo.as_ref() {
        sql.push_str(" AND repo = ?");
        params.push(r);
    }
    if let Some(id) = exclude.as_ref() {
        sql.push_str(" AND id <> ?");
        params.push(id);
    }
    sql.push_str(" ORDER BY id ASC");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params.as_slice(), |r| {
            Ok(SimhashRow {
                id: r.get(0)?,
                simhash: r.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
#[path = "tests/simhash_scan.rs"]
mod tests;
