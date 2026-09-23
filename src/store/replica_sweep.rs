//! The abandoned-stage sweep: `replica_staged_part` rows of an upload whose
//! last part never arrived, and `code_generation` rows still `staged`.
//! Neither was ever published, so neither is state a peer has seen.
//!
//! What it must NOT touch is why it lives in one place: an `active` or
//! `superseded` generation, its projection, the journal, and every receipt —
//! removing a receipt would turn a peer's retry into a second acceptance.

use rusqlite::Connection;
use time::OffsetDateTime;

use crate::prelude::*;
use crate::store::memory_row;

/// How long an unfinished upload is kept before it counts as abandoned.
///
/// A sender retries a refused part in seconds and re-cuts a whole upload in
/// minutes; a day is generous enough that no live retry is ever swept, and
/// short enough that a repository-sized manifest does not sit in the database
/// for a season after the machine that sent it went away.
pub const ABANDONED_AFTER_HOURS: i64 = 24;

/// What one sweep removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Swept {
    /// `replica_staged_part` rows of uploads that never completed.
    pub parts: u64,
    /// `code_generation` rows still `staged`, with their projection rows.
    pub generations: u64,
}

/// Remove every staged part and staged generation older than
/// [`ABANDONED_AFTER_HOURS`] before `now`.
///
/// # Errors
/// Propagates SQLite failures.
pub fn run(conn: &Connection, now: OffsetDateTime) -> Result<Swept> {
    let cutoff = memory_row::iso_format(now - time::Duration::hours(ABANDONED_AFTER_HOURS))?;
    let parts = conn.execute(
        "DELETE FROM replica_staged_part WHERE created_at < ?1",
        [&cutoff],
    )?;
    // The projection goes with the generation that owns it, and only for a
    // generation this sweep is about to remove — a join, never a scan for
    // orphans, so a row whose generation survives cannot be caught by it.
    for table in ["remote_code_file", "remote_code_symbol", "remote_code_edge"] {
        conn.execute(
            &format!(
                "DELETE FROM {table} WHERE (repo, generation_id) IN \
                 (SELECT repo, generation_id FROM code_generation \
                   WHERE state = 'staged' AND created_at < ?1)"
            ),
            [&cutoff],
        )?;
    }
    let generations = conn.execute(
        "DELETE FROM code_generation WHERE state = 'staged' AND created_at < ?1",
        [&cutoff],
    )?;
    Ok(Swept {
        parts: u64::try_from(parts).unwrap_or(0),
        generations: u64::try_from(generations).unwrap_or(0),
    })
}

#[cfg(test)]
#[path = "tests/replica_sweep.rs"]
mod tests;
