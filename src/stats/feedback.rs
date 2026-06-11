//! Per-memory feedback counters: `used` and `irrelevant`.
//!
//! Each record corresponds to one memory id and tracks how many times the
//! memory was surfaced and accepted vs. dismissed. Inserts use SQLite UPSERT
//! so callers do not need to seed rows.

use rusqlite::Connection;
use time::OffsetDateTime;

use crate::prelude::*;
use crate::stats::sqlite::StatsDb;
use crate::store::memory_row;

/// Validate the `q-<yyyymmdd>-<8hex>` query-id shape emitted by
/// `retrieval::pipeline`. Shared by `comemory feedback` (reject typos
/// loudly) and tests.
pub fn is_valid_query_id(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 19
        && s.starts_with("q-")
        && b[2..10].iter().all(u8::is_ascii_digit)
        && b[10] == b'-'
        && b[11..19]
            .iter()
            .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

/// Upsert the `used` side of the per-memory counter row: insert with
/// `used_count = 1` or bump the existing count, refreshing `last_used`
/// to `now` either way. Shared by [`Feedback::record_used`] and
/// [`record_with_provenance`] so the UPSERT SQL exists exactly once.
/// Accepts any [`Connection`] (a `rusqlite::Transaction` derefs to one).
pub(crate) fn upsert_used(conn: &Connection, id: &str, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count, last_used)
             VALUES (?1, 1, 0, ?2)
             ON CONFLICT(memory_id) DO UPDATE SET used_count = used_count + 1, last_used = ?2",
        rusqlite::params![id, now],
    )?;
    Ok(())
}

/// Upsert the `irrelevant` side of the per-memory counter row: insert with
/// `irrelevant_count = 1` or bump the existing count. `last_used` is left
/// untouched — a dismissal is not a use. Shared by
/// [`Feedback::record_irrelevant`] and [`record_with_provenance`].
pub(crate) fn upsert_irrelevant(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count)
             VALUES (?1, 0, 1)
             ON CONFLICT(memory_id) DO UPDATE SET irrelevant_count = irrelevant_count + 1",
        rusqlite::params![id],
    )?;
    Ok(())
}

/// Record a batch of used/irrelevant verdicts for one query in a single
/// transaction: one `feedback_events` row per id plus the matching
/// counter upserts. All-or-nothing — a failure on any id leaves both
/// tables untouched, so events and counters cannot drift.
///
/// The query id is recorded verbatim; it is not required to exist in
/// `retrieval_log` (gc may have evicted the row, or the caller may be
/// replaying feedback) — the caller decides whether to warn.
///
/// `feedback_events.at` goes through [`memory_row::iso_format`] — the
/// same writer as `retrieval_log.at` — so gc's lexicographic cutoff
/// comparison stays sound across both tables.
pub fn record_with_provenance(
    db: &mut StatsDb,
    query_id: &str,
    used: &[String],
    irrelevant: &[String],
) -> Result<()> {
    let now = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let tx = db.conn_mut().transaction()?;
    for (ids, verdict) in [(used, "used"), (irrelevant, "irrelevant")] {
        for id in ids {
            tx.execute(
                "INSERT INTO feedback_events(query_id, memory_id, verdict, at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![query_id, id, verdict, now],
            )?;
        }
    }
    for id in used {
        upsert_used(&tx, id, &now)?;
    }
    for id in irrelevant {
        upsert_irrelevant(&tx, id)?;
    }
    tx.commit()?;
    Ok(())
}

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
    ///
    /// SQLite errors propagate through `Error::Sqlite` so the CLI surfaces
    /// them with `EX_SOFTWARE` and a sqlite-tagged message — matching the
    /// `stats::sqlite` module's recent shift away from `Error::Other` wraps.
    pub fn record_used(&self, id: &str) -> Result<()> {
        let now = memory_row::iso_format(OffsetDateTime::now_utc())?;
        upsert_used(self.db.conn(), id, &now)
    }

    /// Record that memory `id` was surfaced but judged irrelevant. Increments
    /// `irrelevant_count`; does not touch `last_used`.
    ///
    /// SQLite errors propagate through `Error::Sqlite` for the same reason
    /// as [`Self::record_used`].
    pub fn record_irrelevant(&self, id: &str) -> Result<()> {
        upsert_irrelevant(self.db.conn(), id)
    }

    /// Look up `(used_count, irrelevant_count)` for memory `id`. Returns
    /// `(0, 0)` only when no row exists; any other SQLite failure (missing
    /// table, locked db, corrupted file) is surfaced as `Error::Sqlite` so
    /// downstream consumers (e.g. `prune::low_value`) cannot be silently fed
    /// zeroed counts and the CLI exit code stays consistent with the rest of
    /// the stats layer.
    pub fn counts(&self, id: &str) -> Result<(u64, u64)> {
        match self.db.conn().query_row(
            "SELECT used_count, irrelevant_count FROM feedback WHERE memory_id = ?1",
            rusqlite::params![id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        ) {
            Ok((u, i)) => Ok((u as u64, i as u64)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok((0, 0)),
            Err(e) => Err(Error::Sqlite(e)),
        }
    }
}
