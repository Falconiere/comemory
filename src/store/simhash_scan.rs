//! Bulk `(id, simhash)` scan over live memories.
//!
//! The near-dup surfaces all need the same thing: every live memory's stored
//! fingerprint, optionally narrowed to one repo, optionally minus one row.
//! `save` wants it to find the single closest neighbor of the body being
//! written; `consolidate` wants it to cluster the whole corpus. One query
//! serves both (Binding Rule 1) instead of each growing its own copy.

use rusqlite::Connection;

use super::{
    orm,
    schema_memory::{Memories, memories},
};
use crate::prelude::*;
use toolu_orm::core::query_column::CommonOps;

/// One live memory's stored SimHash fingerprint.
///
/// `simhash` is the raw `INTEGER` column: the write path stores
/// `simhash::of_body(body) as i64`, so a reader casts back with `as u64`
/// before handing it to [`crate::utilities::simhash::hamming64`].
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
    let mut query = Memories::select()
        .columns_typed(&[&memories::id, &memories::simhash])
        .filter(memories::deleted_at.is_null())
        .order_by(memories::id.asc());
    if let Some(repo) = repo {
        query = query.filter(memories::repo.eq(repo));
    }
    if let Some(id) = exclude {
        query = query.filter(memories::id.ne(id));
    }
    orm::query_all(conn, query.to_sql(), |r| {
        Ok(SimhashRow {
            id: r.get(0)?,
            simhash: r.get(1)?,
        })
    })
}

#[cfg(test)]
#[path = "tests/simhash_scan.rs"]
mod tests;
