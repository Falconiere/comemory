//! The corpus counters behind `comemory stats` / `GET /api/v1/stats`: a
//! generic scoped `COUNT(*)`, a table-wide `COUNT(*)`, and the logical
//! database size. Moved out of `api::stats` (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).

use rusqlite::Connection;

use crate::prelude::*;

/// `COUNT(*)` over `table` under `predicate`, narrowed to `repo` when one
/// was requested. `table` and `predicate` are `&'static str`: interpolated
/// into the SQL, so only a compile-time literal at the call site can reach
/// them — `repo`, the one genuinely dynamic value, is bound as a parameter.
pub fn scoped_count(
    conn: &Connection,
    table: &'static str,
    predicate: &'static str,
    repo: Option<&str>,
) -> Result<u64> {
    if let Some(repo) = repo {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE {predicate} AND repo = ?1");
        Ok(conn.query_row(&sql, [repo], |r| r.get::<_, i64>(0))? as u64)
    } else {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE {predicate}");
        Ok(conn.query_row(&sql, [], |r| r.get::<_, i64>(0))? as u64)
    }
}

/// `COUNT(*)` over the whole of `table`, no predicate. `table` is
/// `&'static str` for the same reason as [`scoped_count`]'s.
pub fn count_table(conn: &Connection, table: &'static str) -> Result<u64> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    Ok(conn.query_row(&sql, [], |r| r.get::<_, i64>(0))? as u64)
}

/// `page_count * page_size` — the logical size of the database (not the
/// file's length on disk: WAL-mode pages not yet checkpointed live in
/// `comemory.db-wal` and the two legitimately disagree).
pub fn db_bytes(conn: &Connection) -> Result<u64> {
    let pages: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    let size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    Ok((pages.max(0) as u64).saturating_mul(size.max(0) as u64))
}

#[cfg(test)]
#[path = "tests/stats_counts.rs"]
mod tests;
