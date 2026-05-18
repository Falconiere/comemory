//! Per-memory feedback counters: `used` and `irrelevant`.
//!
//! Each record corresponds to one memory id and tracks how many times the
//! memory was surfaced and accepted vs. dismissed. Inserts use SQLite UPSERT
//! so callers do not need to seed rows.

use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;

use crate::prelude::*;
use crate::stats::sqlite::StatsDb;

/// Thin handle that borrows a [`StatsDb`] and exposes feedback operations.
/// The borrow is `&mut` per the plan; rusqlite uses interior mutability so the
/// individual operations only need `&self`.
pub struct Feedback<'a> {
    db: &'a mut StatsDb,
}

impl<'a> Feedback<'a> {
    /// Borrow `db` for the duration of feedback operations.
    pub fn new(db: &'a mut StatsDb) -> Self {
        Self { db }
    }

    /// Record that memory `id` was used in a response. Increments `used_count`
    /// and refreshes `last_used` to the current UTC time.
    pub fn record_used(&self, id: &str) -> Result<()> {
        let now = OffsetDateTime::now_utc()
            .format(&Iso8601::DEFAULT)
            .map_err(|e| Error::Other(e.to_string()))?;
        self.db
            .conn()
            .execute(
                "INSERT INTO feedback(memory_id, used_count, irrelevant_count, last_used)
                 VALUES (?1, 1, 0, ?2)
                 ON CONFLICT(memory_id) DO UPDATE SET used_count = used_count + 1, last_used = ?2",
                rusqlite::params![id, now],
            )
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }

    /// Record that memory `id` was surfaced but judged irrelevant. Increments
    /// `irrelevant_count`; does not touch `last_used`.
    pub fn record_irrelevant(&self, id: &str) -> Result<()> {
        self.db
            .conn()
            .execute(
                "INSERT INTO feedback(memory_id, used_count, irrelevant_count)
                 VALUES (?1, 0, 1)
                 ON CONFLICT(memory_id) DO UPDATE SET irrelevant_count = irrelevant_count + 1",
                rusqlite::params![id],
            )
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }

    /// Look up `(used_count, irrelevant_count)` for memory `id`. Returns
    /// `(0, 0)` only when no row exists; any other SQLite failure (missing
    /// table, locked db, corrupted file) is surfaced as `Error::Other` so
    /// downstream consumers (e.g. `prune::low_value`) cannot be silently fed
    /// zeroed counts.
    pub fn counts(&self, id: &str) -> Result<(u64, u64)> {
        match self.db.conn().query_row(
            "SELECT used_count, irrelevant_count FROM feedback WHERE memory_id = ?1",
            rusqlite::params![id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        ) {
            Ok((u, i)) => Ok((u as u64, i as u64)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok((0, 0)),
            Err(e) => Err(Error::Other(format!("feedback counts: {e}"))),
        }
    }
}
