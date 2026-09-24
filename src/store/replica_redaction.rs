//! Redaction of shared events' journal copies (#254): retention **expires**
//! them, a memory purge **erases** them.
//!
//! Both blank `replica_payload.bytes` and keep the row, so the digest — and
//! with it the feed position, the revision and every receipt — survives as
//! the metadata a retry or a repair is deduplicated by. What differs is the
//! answer a later offer of the same event reads: `payload_expired` or
//! `payload_erased` ([`Redaction`]).
//!
//! Hand SQL: both statements select their digests through `IN` subqueries
//! across the feed, the revisions and the two event tables, which the
//! declared builders do not express; tracked in `docs/guides/runtime-orm.md`.

use rusqlite::{Connection, params};

use super::replica_read::Redaction;
use crate::prelude::*;
use crate::utilities::telemetry::entity::{ACTIVITY_EVENT, FEEDBACK_EVENT};
use crate::utilities::telemetry::target;

/// Which shared events' journal copies one [`redact`] call reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach<'a> {
    /// Retention: every shared event older than the cutoff — its materialized
    /// row is about to be evicted (its `event_id` is on a `feedback_events` or
    /// `activity_log` row with `at` before it), or its own feed position is
    /// older, the arm that reaches an import this machine never materialized.
    /// Must run BEFORE the eviction deletes those rows, or their ids are gone.
    /// Copies are **expired**; one already redacted either way is left as is.
    PastRetention(&'a str),
    /// Purge: every shared verdict on this memory id, before its
    /// `feedback_events` rows are deleted. Copies are **erased**, and an
    /// expired one is upgraded — the purge is the stronger claim a replay must
    /// read.
    VerdictsOn(&'a str),
}

/// The digests [`Reach::PastRetention`] selects: `?3` the cutoff, `?4` and `?5`
/// the two event kinds.
const PAST_RETENTION: &str = "
        SELECT payload_digest FROM replica_feed
         WHERE entity_kind IN (?4, ?5) AND at < ?3 AND payload_digest IS NOT NULL
        UNION
        SELECT payload_digest FROM replica_revision
         WHERE entity_kind = ?4 AND entity_key IN (
               SELECT event_id FROM feedback_events WHERE at < ?3 AND event_id IS NOT NULL)
        UNION
        SELECT payload_digest FROM replica_revision
         WHERE entity_kind = ?5 AND entity_key IN (
               SELECT event_id FROM activity_log WHERE at < ?3 AND event_id IS NOT NULL)";

/// The digests [`Reach::VerdictsOn`] selects: `?3` the memory id, `?4` the
/// verdict kind, `?5` the memory target kind.
const VERDICTS_ON: &str = "
        SELECT payload_digest FROM replica_revision
         WHERE entity_kind = ?4 AND entity_key IN (
               SELECT event_id FROM feedback_events
                WHERE memory_id = ?3 AND target_kind = ?5 AND event_id IS NOT NULL)";

/// Blank the bytes of every journal copy `reach` selects, in one statement
/// however many events it names, and stamp why. Returns the payloads redacted.
///
/// # Errors
/// Propagates SQLite failures.
pub fn redact(conn: &Connection, reach: Reach<'_>, at: &str) -> Result<u64> {
    let (redaction, guard, selected, keys) = match reach {
        Reach::PastRetention(cutoff) => (
            Redaction::Expired,
            "redacted_at IS NULL".to_string(),
            PAST_RETENTION,
            [cutoff, FEEDBACK_EVENT, ACTIVITY_EVENT],
        ),
        Reach::VerdictsOn(memory_id) => (
            Redaction::Erased,
            format!(
                "redacted_at IS NULL OR redaction = '{}'",
                Redaction::Expired.as_str()
            ),
            VERDICTS_ON,
            [memory_id, FEEDBACK_EVENT, target::MEMORY],
        ),
    };
    let redacted = conn.execute(
        &format!(
            "UPDATE replica_payload
                SET bytes = NULL, redacted_at = COALESCE(redacted_at, ?1), redaction = ?2
              WHERE ({guard}) AND digest IN ({selected})"
        ),
        params![at, redaction.as_str(), keys[0], keys[1], keys[2]],
    )?;
    Ok(u64::try_from(redacted).unwrap_or(0))
}

/// Whether — and why — the bytes behind `digest` are gone: `None` for a
/// digest that still has its bytes and for one this engine never stored.
/// Acceptance reads it to pick `payload_erased` or `payload_expired`.
///
/// # Errors
/// Propagates SQLite failures.
pub fn redaction_of(conn: &Connection, digest: &str) -> Result<Option<Redaction>> {
    let mut statement = conn
        .prepare_cached("SELECT redacted_at, redaction FROM replica_payload WHERE digest = ?1")?;
    let mut rows = statement.query([digest])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let (at, kind): (Option<String>, Option<String>) = (row.get(0)?, row.get(1)?);
    Ok(Redaction::of(at.as_deref(), kind.as_deref()))
}

#[cfg(test)]
#[path = "tests/replica_redaction.rs"]
mod tests;
