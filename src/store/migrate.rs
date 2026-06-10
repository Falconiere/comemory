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
pub const CURRENT_VERSION: &str = "4";

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

/// Apply all pending migrations. Safe to re-run; each migration is
/// only applied if its key is absent from `schema_meta`.
pub fn run(conn: &mut Connection) -> Result<()> {
    apply(conn, "0001_schema_meta", M_BOOTSTRAP)?;
    apply(conn, "0002_v2_tables", M_V2)?;
    apply(conn, "0003_stats_tables", M_V3)?;
    apply(conn, "0004_v4_rank", M_V4)?;
    backfill_memory_simhash(conn)?;
    set_version(conn, CURRENT_VERSION)?;
    Ok(())
}

/// Apply one migration if it has not yet been recorded in
/// `schema_meta`; returns `true` when the SQL was newly executed. The
/// bootstrap migration is a special case: it creates `schema_meta`
/// itself, so we cannot read from it before the migration runs. The SQL
/// uses `CREATE TABLE IF NOT EXISTS`, which is idempotent on its own —
/// but it therefore always reports `true`, so never hang a
/// run-exactly-once hook off the bootstrap's return value.
fn apply(conn: &mut Connection, key: &str, sql: &str) -> Result<bool> {
    if key == "0001_schema_meta" {
        conn.execute_batch(sql)
            .map_err(|e| Error::Migration(format!("{key}: {e}")))?;
        return Ok(true);
    }
    let already: Option<String> = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .ok();
    if already.is_some() {
        return Ok(false);
    }
    let tx = conn.transaction()?;
    tx.execute_batch(sql)
        .map_err(|e| Error::Migration(format!("{key}: {e}")))?;
    tx.execute("INSERT INTO schema_meta(key, value) VALUES(?1, '1')", [key])?;
    tx.commit()?;
    Ok(true)
}

/// Compute and store simhash for every memory that still has the
/// DEFAULT 0 placeholder left by the v4 migration. Runs unconditionally
/// on every [`run`] so a crash between the migration commit and the
/// backfill heals on the next open; once every row is hashed the
/// `WHERE simhash = 0` scan returns nothing and this is a no-op. All
/// updates commit in one transaction so a partial backfill never
/// persists.
fn backfill_memory_simhash(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("SELECT id, body FROM memories WHERE simhash = 0")?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        drop(stmt);
        for (id, body) in rows {
            let hash = crate::simhash::of_body(&body);
            // SQLite INTEGER is i64; store the u64 bit pattern.
            tx.execute(
                "UPDATE memories SET simhash = ?1 WHERE id = ?2",
                rusqlite::params![hash as i64, id],
            )?;
        }
    }
    tx.commit()?;
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
