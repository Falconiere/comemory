//! Redaction of journal copies: retention **expires** shared events (#254);
//! a memory purge **erases** the verdicts on it; a permanent erase (#256)
//! erases an entity's own payloads and the shared runs naming it.
//!
//! Every arm blanks `replica_payload.bytes` and keeps the row: the digest,
//! feed position, revision and receipts survive as the barrier a later offer
//! reads — `payload_expired` or `payload_erased` ([`Redaction`]).
//!
//! Hand SQL: `IN` subqueries across the feed, revisions, outbox and event
//! tables; tracked in `docs/guides/runtime-orm.md`.

use rusqlite::{Connection, params_from_iter};

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
    /// Permanent erase of one entity `(kind, key)`: every payload its own feed
    /// rows, its revision and its outbox rows name. Erased, upgrading expiry.
    Entity(&'a str, &'a str),
    /// Permanent erase of a memory id: every shared run whose `activity_log`
    /// summary names it (`$.id`), and every held run payload whose summary
    /// does — a run evicted locally still has its journal copy. Erased,
    /// upgrading expiry.
    RunsNaming(&'a str),
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

/// The digests [`Reach::Entity`] selects: `?3` the kind, `?4` the key.
const ENTITY: &str = "
        SELECT payload_digest FROM replica_feed
         WHERE entity_kind = ?3 AND entity_key = ?4 AND payload_digest IS NOT NULL
        UNION
        SELECT payload_digest FROM replica_revision
         WHERE entity_kind = ?3 AND entity_key = ?4 AND payload_digest IS NOT NULL
        UNION
        SELECT payload_digest FROM replica_operation
         WHERE entity_kind = ?3 AND entity_key = ?4 AND payload_digest IS NOT NULL";

/// The digests [`Reach::RunsNaming`] selects: `?3` the memory id, `?4` the
/// activity kind. Each `json_extract` sits behind a `json_valid` in a `CASE`,
/// the one form SQLite guarantees to short-circuit, so a malformed summary is
/// skipped rather than failing the statement.
const RUNS_NAMING: &str = "
        SELECT payload_digest FROM replica_revision
         WHERE entity_kind = ?4 AND payload_digest IS NOT NULL AND entity_key IN (
               SELECT event_id FROM activity_log
                WHERE event_id IS NOT NULL
                  AND CASE WHEN json_valid(summary)
                           THEN json_extract(summary, '$.id') = ?3 ELSE 0 END)
        UNION
        SELECT digest FROM replica_payload
         WHERE entity_kind = ?4 AND bytes IS NOT NULL
           AND CASE WHEN json_valid(bytes)
                    THEN json_extract(bytes, '$.summary.id') = ?3 ELSE 0 END";

/// The guard every erasing arm shares: a copy not yet redacted, or one only
/// expired — an erasure is the stronger claim a replay must read.
fn erasable() -> String {
    format!(
        "redacted_at IS NULL OR redaction = '{}'",
        Redaction::Expired.as_str()
    )
}

/// A digest a pending outbox operation still owes upstream is exempt from
/// retention: blanking it here would push a `NULL` payload the next time
/// that operation sends. The erasing arms carry no such exemption: a
/// permanent erase withdraws the entity's pending upserts and restores
/// itself before reaching redaction (#256, B-5), and the only pending
/// operation it leaves — the tombstone — names no payload.
const NOT_OWED: &str = "digest NOT IN (\
     SELECT payload_digest FROM replica_operation \
      WHERE state = 'pending' AND payload_digest IS NOT NULL)";

/// Blank the bytes of every journal copy `reach` selects, in one statement
/// however many events it names, and stamp why. Returns the digests redacted
/// — callers reach beyond `replica_payload` with them, e.g. a killed
/// replay's [`super::replica_redaction_copies`] scratch copy.
///
/// # Errors
/// Propagates SQLite failures.
pub fn redact(conn: &Connection, reach: Reach<'_>, at: &str) -> Result<Vec<String>> {
    let (redaction, guard, selected, keys): (_, _, _, Vec<&str>) = match reach {
        Reach::PastRetention(cutoff) => (
            Redaction::Expired,
            format!("redacted_at IS NULL AND {NOT_OWED}"),
            PAST_RETENTION,
            vec![cutoff, FEEDBACK_EVENT, ACTIVITY_EVENT],
        ),
        Reach::VerdictsOn(memory_id) => (
            Redaction::Erased,
            erasable(),
            VERDICTS_ON,
            vec![memory_id, FEEDBACK_EVENT, target::MEMORY],
        ),
        Reach::Entity(kind, key) => (Redaction::Erased, erasable(), ENTITY, vec![kind, key]),
        Reach::RunsNaming(memory_id) => (
            Redaction::Erased,
            erasable(),
            RUNS_NAMING,
            vec![memory_id, ACTIVITY_EVENT],
        ),
    };
    let mut statement = conn.prepare(&format!(
        "UPDATE replica_payload
            SET bytes = NULL, redacted_at = COALESCE(redacted_at, ?1), redaction = ?2
          WHERE ({guard}) AND digest IN ({selected})
          RETURNING digest"
    ))?;
    let bound = [at, redaction.as_str()].into_iter().chain(keys);
    let digests = statement
        .query_map(params_from_iter(bound), |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(digests)
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
