//! Whether `comemory.db` is ready for the resident coordinator, asked without
//! creating, migrating or writing it: the coordinator never opens a database
//! a writable command has not already brought to this build's schema.

use std::path::Path;

use serde::Serialize;

use crate::prelude::*;
use crate::store::connection::open_read_only;
use crate::store::migrate::preflight;

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
    let conn = open_read_only(db_path)?;
    if !preflight::schema_meta_exists(&conn)? {
        return Ok(StoreReadiness::MigrationPending);
    }
    let applied = preflight::applied_keys(&conn)?;
    let expected = preflight::expected_markers();
    if applied.iter().any(|k| !expected.contains(k.as_str())) {
        return Ok(StoreReadiness::TooNew);
    }
    if applied.len() == expected.len() {
        return Ok(StoreReadiness::Ready);
    }
    Ok(StoreReadiness::MigrationPending)
}

/// How many migration markers a fully migrated database holds.
#[must_use]
pub fn expected_marker_count() -> usize {
    preflight::expected_markers().len()
}

#[cfg(test)]
#[path = "tests/readiness.rs"]
mod tests;
