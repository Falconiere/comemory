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

/// Expire the journal copy of every shared event older than `cutoff`.
///
/// An event qualifies when its materialized row is about to be evicted (its
/// `event_id` is on a `feedback_events` / `activity_log` row with `at` before
/// `cutoff`), or when its own feed position is older than `cutoff` — the arm
/// that reaches an imported event this machine never materialized. Must run
/// BEFORE the eviction deletes those rows, or their ids are gone. A payload
/// already redacted either way is left as it is. Returns the payloads
/// expired.
///
/// # Errors
/// Propagates SQLite failures.
pub fn expire_events_before(conn: &Connection, cutoff: &str, at: &str) -> Result<u64> {
    let expired = conn.execute(
        "UPDATE replica_payload
            SET bytes = NULL, redacted_at = ?2, redaction = ?5
          WHERE redacted_at IS NULL AND digest IN (
                SELECT payload_digest FROM replica_feed
                 WHERE entity_kind IN (?3, ?4) AND at < ?1 AND payload_digest IS NOT NULL
                UNION
                SELECT payload_digest FROM replica_revision
                 WHERE entity_kind = ?3 AND entity_key IN (
                       SELECT event_id FROM feedback_events
                        WHERE at < ?1 AND event_id IS NOT NULL)
                UNION
                SELECT payload_digest FROM replica_revision
                 WHERE entity_kind = ?4 AND entity_key IN (
                       SELECT event_id FROM activity_log
                        WHERE at < ?1 AND event_id IS NOT NULL))",
        params![
            cutoff,
            at,
            FEEDBACK_EVENT,
            ACTIVITY_EVENT,
            Redaction::Expired.as_str()
        ],
    )?;
    Ok(u64::try_from(expired).unwrap_or(0))
}

/// Erase the journal copies of the verdicts named by `event_ids` — the ones a
/// memory purge is deleting. An expired copy is upgraded to erased: the purge
/// is the stronger claim, and a replay must read it. Returns the payloads
/// erased.
///
/// # Errors
/// Propagates SQLite failures.
pub fn erase_verdicts(conn: &Connection, event_ids: &[String], at: &str) -> Result<u64> {
    let mut erased = 0_u64;
    for event_id in event_ids {
        let changed = conn.execute(
            "UPDATE replica_payload
                SET bytes = NULL, redacted_at = COALESCE(redacted_at, ?3), redaction = ?4
              WHERE (redacted_at IS NULL OR redaction = ?5) AND digest IN (
                    SELECT payload_digest FROM replica_revision
                     WHERE entity_kind = ?1 AND entity_key = ?2)",
            params![
                FEEDBACK_EVENT,
                event_id,
                at,
                Redaction::Erased.as_str(),
                Redaction::Expired.as_str()
            ],
        )?;
        erased += u64::try_from(changed).unwrap_or(0);
    }
    Ok(erased)
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
