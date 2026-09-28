//! Whether `comemory.db` is ready for the resident coordinator, asked without
//! creating, migrating or writing it: the coordinator never opens a database
//! a writable command has not already brought to this build's schema.

use std::path::Path;

use serde::Serialize;

use crate::prelude::*;
use rusqlite::OpenFlags;

use crate::store::Connection;
use crate::store::connection::open_read_only;
use crate::store::migrate::preflight;
use crate::store::store_health;

/// The database's state as the coordinator sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreReadiness {
    /// No `comemory.db` yet.
    Absent,
    /// Every migration this build knows is applied, and nothing newer.
    Ready,
    /// Some migration is still pending; a writable command applies it.
    MigrationPending,
    /// Some migration is still pending and this build's last long-lived
    /// open failed applying it ([`store_health`]).
    ///
    /// [`store_health`]: crate::store::store_health
    MigrationFailed,
    /// A newer build wrote it; this build must not touch it.
    TooNew,
}

/// Probe `db_path` read-only.
///
/// # Errors
/// The file exists but is not a readable SQLite database.
pub fn probe(db_path: &Path) -> Result<StoreReadiness> {
    if !db_path.exists() {
        return Ok(StoreReadiness::Absent);
    }
    let conn = open_probe(db_path)?;
    if !preflight::schema_meta_exists(&conn)? {
        return Ok(pending(db_path));
    }
    let applied = preflight::applied_keys(&conn)?;
    let expected = preflight::expected_markers();
    if applied.iter().any(|k| !expected.contains(k.as_str())) {
        return Ok(StoreReadiness::TooNew);
    }
    if applied.len() == expected.len() {
        return Ok(StoreReadiness::Ready);
    }
    Ok(pending(db_path))
}

/// Open `db_path` read-only for a probe. A WAL database in a directory this
/// process cannot write cannot be read that way — SQLite must create its
/// `-shm` — and answers `SQLITE_READONLY_DIRECTORY`; since no writer can
/// open it there either, it is then read as immutable instead.
///
/// # Errors
/// The file is not a readable SQLite database.
pub fn open_probe(db_path: &Path) -> Result<Connection> {
    let conn = open_read_only(db_path)?;
    match conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    }) {
        Ok(_) => Ok(conn),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.extended_code == rusqlite::ffi::SQLITE_READONLY_DIRECTORY =>
        {
            Ok(Connection::open_with_flags(
                immutable_uri(db_path),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
            )?)
        }
        Err(e) => Err(e.into()),
    }
}

/// `file:<path>?immutable=1`, escaping the characters a URI path cannot
/// hold literally.
fn immutable_uri(db_path: &Path) -> String {
    let mut uri = String::from("file:");
    for c in db_path.to_string_lossy().chars() {
        match c {
            '%' => uri.push_str("%25"),
            '?' => uri.push_str("%3f"),
            '#' => uri.push_str("%23"),
            c => uri.push(c),
        }
    }
    uri.push_str("?immutable=1");
    uri
}

/// Pending — or failed, when this build recorded a failed open.
fn pending(db_path: &Path) -> StoreReadiness {
    if store_health::failed_migration(db_path).is_some() {
        StoreReadiness::MigrationFailed
    } else {
        StoreReadiness::MigrationPending
    }
}

/// How many migration markers a fully migrated database holds.
#[must_use]
pub fn expected_marker_count() -> usize {
    preflight::expected_markers().len()
}

#[cfg(test)]
#[path = "tests/readiness.rs"]
mod tests;
