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
pub const CURRENT_VERSION: &str = "3";

const M_BOOTSTRAP: &str = include_str!("./sql/0001_schema_meta.sql");
const M_V2: &str = include_str!("./sql/0002_v2_tables.sql");
const M_V3: &str = include_str!("./sql/0003_stats_tables.sql");

/// Apply all pending migrations. Safe to re-run; each migration is
/// only applied if its key is absent from `schema_meta`.
pub fn run(conn: &mut Connection) -> Result<()> {
    apply(conn, "0001_schema_meta", M_BOOTSTRAP)?;
    apply(conn, "0002_v2_tables", M_V2)?;
    apply(conn, "0003_stats_tables", M_V3)?;
    set_version(conn, CURRENT_VERSION)?;
    Ok(())
}

/// Apply one migration if it has not yet been recorded in
/// `schema_meta`. The bootstrap migration is a special case: it
/// creates `schema_meta` itself, so we cannot read from it before the
/// migration runs. The SQL uses `CREATE TABLE IF NOT EXISTS`, which is
/// idempotent on its own.
fn apply(conn: &mut Connection, key: &str, sql: &str) -> Result<()> {
    if key == "0001_schema_meta" {
        conn.execute_batch(sql)
            .map_err(|e| Error::Migration(format!("{key}: {e}")))?;
        return Ok(());
    }
    let already: Option<String> = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .ok();
    if already.is_some() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    tx.execute_batch(sql)
        .map_err(|e| Error::Migration(format!("{key}: {e}")))?;
    tx.execute("INSERT INTO schema_meta(key, value) VALUES(?1, '1')", [key])?;
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
