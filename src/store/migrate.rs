//! Schema versioning + migrations.
//!
//! Each migration is an immutable SQL string keyed by its version
//! string. The applied set is tracked in `schema_meta` so that re-runs
//! are idempotent and partial upgrades resume cleanly. The bootstrap
//! migration (`0001_schema_meta`) creates the `schema_meta` table
//! itself, so it is wrapped with `CREATE TABLE IF NOT EXISTS` and runs
//! outside the apply-once gate.

use rusqlite::Connection;

use crate::prelude::*;

/// Highest schema version known to this build. Bumped each time a new
/// migration file is added under `src/store/sql/`.
pub const CURRENT_VERSION: &str = "5";

/// 0001 bootstrap SQL (`schema_meta` table). Public so tests can replay
/// historical schema states exactly as an old binary created them.
pub const M_BOOTSTRAP: &str = include_str!("./sql/0001_schema_meta.sql");
/// 0002 SQL (core v2 tables). Public so tests can replay historical
/// schema states exactly as an old binary created them.
pub const M_V2: &str = include_str!("./sql/0002_v2_tables.sql");
/// 0003 SQL (stats tables). Public so tests can replay historical
/// schema states exactly as an old binary created them.
pub const M_V3: &str = include_str!("./sql/0003_stats_tables.sql");
/// 0004 SQL: access-tracking columns, `memories.simhash` placeholder,
/// and the identifier-tokenized FTS rebuild.
pub const M_V4: &str = include_str!("./sql/0004_v4_rank.sql");
/// 0005 SQL: learning-loop tables (feedback_events, query_expansions),
/// retrieval_log.duration_ms, and the search_stats drop.
pub const M_V5: &str = include_str!("./sql/0005_v5_learning.sql");

/// Apply all pending migrations. Safe to re-run; each migration is
/// only applied if its key is absent from `schema_meta`.
pub fn run(conn: &mut Connection) -> Result<()> {
    apply(conn, "0001_schema_meta", M_BOOTSTRAP)?;
    apply(conn, "0002_v2_tables", M_V2)?;
    apply(conn, "0003_stats_tables", M_V3)?;
    apply(conn, "0004_v4_rank", M_V4)?;
    backfill_memory_simhash(conn)?;
    apply(conn, "0005_v5_learning", M_V5)?;
    rehash_simhashes(conn)?;
    set_version(conn, CURRENT_VERSION)?;
    Ok(())
}

/// Apply one migration if it has not yet been recorded in
/// `schema_meta`. The bootstrap migration is a special case: it creates
/// `schema_meta` itself, so we cannot read from it before the migration
/// runs; its SQL uses `CREATE TABLE IF NOT EXISTS`, which is idempotent
/// on its own.
fn apply(conn: &mut Connection, key: &str, sql: &str) -> Result<()> {
    if key == "0001_schema_meta" {
        conn.execute_batch(sql)
            .map_err(|e| Error::Migration(format!("{key}: {e}")))?;
        return Ok(());
    }
    if marker_done(conn, key) {
        return Ok(());
    }
    let tx = conn.transaction()?;
    tx.execute_batch(sql)
        .map_err(|e| Error::Migration(format!("{key}: {e}")))?;
    insert_marker(&tx, key)?;
    tx.commit()?;
    Ok(())
}

/// True when the run-once marker `key` is already recorded in
/// `schema_meta`. Shared by [`apply`] and the simhash backfill/rehash
/// passes so every run-once gate reads the marker identically.
fn marker_done(conn: &Connection, key: &str) -> bool {
    conn.query_row(
        "SELECT value FROM schema_meta WHERE key = ?1",
        [key],
        |row| row.get::<_, String>(0),
    )
    .is_ok()
}

/// Record the run-once marker `key` inside the caller's transaction so
/// the marker only persists together with the work it gates.
fn insert_marker(tx: &rusqlite::Transaction<'_>, key: &str) -> Result<()> {
    tx.execute("INSERT INTO schema_meta(key, value) VALUES(?1, '1')", [key])?;
    Ok(())
}

/// Compute and store simhash for every memory that still has the
/// DEFAULT 0 placeholder left by the v4 migration. Runs exactly once,
/// keyed '0004_simhash_backfill' in `schema_meta` — the marker insert
/// commits in the same transaction as the updates, so a crash between
/// the v4 apply and the backfill (or mid-backfill) heals on the next
/// open: the marker is absent and the whole pass re-runs. Once the
/// marker is committed, opens skip the `memories` table scan entirely.
fn backfill_memory_simhash(conn: &mut Connection) -> Result<()> {
    if marker_done(conn, "0004_simhash_backfill") {
        return Ok(());
    }
    let tx = conn.transaction()?;
    recompute_simhashes(
        &tx,
        "SELECT id, body FROM memories WHERE simhash = 0",
        "UPDATE memories SET simhash = ?1 WHERE id = ?2",
    )?;
    insert_marker(&tx, "0004_simhash_backfill")?;
    tx.commit()?;
    Ok(())
}

/// Recompute every stored simhash with the M2-aligned `simhash::tokens`
/// (Unicode lowercase + diacritic fold). Runs exactly once, keyed
/// '0005_simhash_rehash' in schema_meta; the marker insert commits in
/// the same transaction as the updates so a crash mid-rehash re-runs
/// the whole pass on the next open (idempotent — the recompute is a
/// pure function of the stored body/snippet).
fn rehash_simhashes(conn: &mut Connection) -> Result<()> {
    if marker_done(conn, "0005_simhash_rehash") {
        return Ok(());
    }
    let tx = conn.transaction()?;
    recompute_simhashes(
        &tx,
        "SELECT id, body FROM memories",
        "UPDATE memories SET simhash = ?1 WHERE id = ?2",
    )?;
    recompute_simhashes(
        &tx,
        "SELECT id, snippet FROM code_symbols",
        "UPDATE code_symbols SET simhash = ?1 WHERE id = ?2",
    )?;
    insert_marker(&tx, "0005_simhash_rehash")?;
    tx.commit()?;
    Ok(())
}

/// Recompute `simhash::of_body` over every `(id, text)` row that
/// `sql_select` yields and persist via `sql_update` (`?1` = hash,
/// `?2` = id). Both statements are prepared once, outside the row loop.
/// The id column is bound as a dynamic [`rusqlite::types::Value`] so
/// one helper serves both `memories` (TEXT id) and `code_symbols`
/// (INTEGER id).
fn recompute_simhashes(
    tx: &rusqlite::Transaction<'_>,
    sql_select: &str,
    sql_update: &str,
) -> Result<()> {
    let mut select = tx.prepare(sql_select)?;
    let mut update = tx.prepare(sql_update)?;
    let rows: Vec<(rusqlite::types::Value, String)> = select
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    for (id, text) in rows {
        // SQLite INTEGER is i64; store the u64 bit pattern.
        let hash = crate::simhash::of_body(&text) as i64;
        update.execute(rusqlite::params![hash, id])?;
    }
    Ok(())
}

/// Upsert the current schema version into `schema_meta`.
fn set_version(conn: &Connection, version: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES('version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [version],
    )?;
    Ok(())
}
