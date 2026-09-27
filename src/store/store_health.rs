//! `store::store_health` — what a failed migration leaves behind (#256,
//! B-8), so a later read-only probe can tell "this build tried and failed"
//! from "no build has tried yet".
//!
//! [`open_recorded`] is [`connection::open`] for the long-lived entry points
//! (`serve`): when the open fails on a database still behind this build, it
//! writes [`RECORD_FILE`] beside the database (`{state, detail, at,
//! binary_version}`); a successful open removes it. The record is written
//! best-effort — a data directory too read-only to hold it is also too
//! read-only to migrate, and still reads as pending, never healthy.
//! [`crate::store::readiness::probe`] reads it through [`failed_migration`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::prelude::*;
use crate::store::readiness::{self, StoreReadiness};
use crate::store::{Connection, connection, memory_row};

/// The record's file name, beside `comemory.db`.
pub const RECORD_FILE: &str = "store-health.json";

/// [`Record::state`] of a migration that failed.
pub const MIGRATION_FAILED: &str = "migration_failed";

/// The failed open, as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// [`MIGRATION_FAILED`].
    pub state: String,
    /// The error the open returned.
    pub detail: String,
    /// When it failed (RFC 3339).
    pub at: String,
    /// The version of the build that failed.
    pub binary_version: String,
}

/// Where the record for `db_path` lives.
#[must_use]
pub fn record_path(db_path: &Path) -> PathBuf {
    db_path.with_file_name(RECORD_FILE)
}

/// The failed-migration record this build left for `db_path`, if any. A
/// record from another build says nothing about this one, so it is ignored.
#[must_use]
pub fn failed_migration(db_path: &Path) -> Option<Record> {
    let bytes = std::fs::read(record_path(db_path)).ok()?;
    serde_json::from_slice::<Record>(&bytes)
        .ok()
        .filter(|r| r.state == MIGRATION_FAILED && r.binary_version == env!("CARGO_PKG_VERSION"))
}

/// Open `db_path` like [`connection::open`], recording a failed migration
/// and clearing the record once an open succeeds.
///
/// # Errors
/// Whatever [`connection::open`] returns; the record is written or removed
/// best-effort and never replaces that error.
pub fn open_recorded(db_path: &Path) -> Result<Connection> {
    match connection::open(db_path) {
        Ok(conn) => {
            if let Err(e) = std::fs::remove_file(record_path(db_path))
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(error = %e, "store health: could not clear the failure record");
            }
            Ok(conn)
        }
        Err(e) => {
            if matches!(
                readiness::probe(db_path),
                Ok(StoreReadiness::MigrationPending | StoreReadiness::MigrationFailed)
            ) && let Err(write) = record(db_path, &e)
            {
                tracing::warn!(error = %write, "store health: could not record the failed migration");
            }
            Err(e)
        }
    }
}

fn record(db_path: &Path, failure: &Error) -> Result<()> {
    let record = Record {
        state: MIGRATION_FAILED.into(),
        detail: failure.to_string(),
        at: memory_row::iso_format(OffsetDateTime::now_utc())?,
        binary_version: env!("CARGO_PKG_VERSION").into(),
    };
    let mut bytes = serde_json::to_vec_pretty(&record)?;
    bytes.push(b'\n');
    std::fs::write(record_path(db_path), bytes)?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/store_health.rs"]
mod tests;
