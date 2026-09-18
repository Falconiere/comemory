//! SQLite-backed stats store: opens `comemory.db` via the shared
//! [`crate::store::connection::open`] so all data lands in the single v0.2
//! database (spec §4: one file).
//!
//! `StatsDb` is the shared connection handle the two feedback recorders open
//! through — [`crate::domains::learning::feedback_tracking::record_with_provenance`]
//! and [`crate::domains::learning::code_feedback::record_code_with_provenance`]
//! borrow [`Self::conn_mut`] for their own transactions. It owns no table:
//! the `index_failures` bookkeeping it once delegated moved wholly to
//! [`crate::store::index_failures`] with #173, leaving the connection itself
//! as this type's only responsibility.

use crate::prelude::*;
use crate::store::Connection;
use crate::store::connection;

/// Owns a SQLite connection to `comemory.db` for stats operations.
pub struct StatsDb {
    conn: Connection,
}

impl StatsDb {
    /// Open (or create) `comemory.db` at `path`, running all pending
    /// migrations so the stats tables are guaranteed to exist. Creates the
    /// parent directory on demand so callers do not need to invoke
    /// [`crate::config::paths::Paths::ensure_dirs`] first.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = connection::open(path)?;
        Ok(Self { conn })
    }

    /// Borrow the underlying connection (read-only access).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Borrow the underlying connection mutably (for transactions).
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

#[cfg(test)]
#[path = "tests/telemetry.rs"]
mod tests;
