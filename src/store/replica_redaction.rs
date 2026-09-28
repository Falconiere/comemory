//! Redaction of journal copies: retention **expires** shared events (#254);
//! a memory purge **erases** the verdicts on it; a permanent erase (#256)
//! erases an entity's own payloads and the shared runs naming it; a merge of
//! the erasure manifest ([`bar`]) erases every digest it lists, and
//! [`erased_entities`] is what the manifest is first written from.
//!
//! Every arm blanks `replica_payload.bytes` and keeps the row: the digest,
//! feed position, revision and receipts survive as the barrier a later offer
//! reads — `payload_expired` or `payload_erased` ([`Redaction`]).
//!
//! Hand SQL: `IN` subqueries across the feed, revisions, outbox and event
//! tables, and `json_each` over a digest list; tracked in
//! `docs/guides/runtime-orm.md`.

use std::collections::HashMap;

use rusqlite::{Connection, params, params_from_iter};

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
    /// A merge of the erasure manifest (#256, B-4): every digest in this JSON
    /// array, whoever's it is. Erased, upgrading expiry.
    Listed(&'a str),
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

/// The digests [`Reach::Listed`] selects: `?3` a JSON array of them.
const LISTED: &str = "SELECT value FROM json_each(?3)";

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
        Reach::Listed(digests) => (Redaction::Erased, erasable(), LISTED, vec![digests]),
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

/// Make every digest in `digests` an erased barrier (#256, B-4): a payload
/// row this engine holds loses its bytes (an expired one is upgraded), and a
/// digest it never stored gets a bytes-less row under `kind` — the row
/// acceptance reads to answer `payload_erased`, so merging the erasure
/// manifest stops a replay even of content this database never held. The
/// barrier row's `schema_version` and `byte_len` are `0`: the manifest does
/// not carry them. Returns how many digests were barred or redacted.
///
/// # Errors
/// Propagates SQLite failures.
pub fn bar(conn: &Connection, digests: &[String], kind: &str, at: &str) -> Result<usize> {
    if digests.is_empty() {
        return Ok(0);
    }
    let listed = serde_json::to_string(digests)?;
    let barred = conn.execute(
        "INSERT OR IGNORE INTO replica_payload
             (digest, entity_kind, schema_version, bytes, byte_len, created_at,
              redacted_at, redaction)
         SELECT value, ?2, 0, NULL, 0, ?3, ?3, ?4 FROM json_each(?1)",
        params![listed, kind, at, Redaction::Erased.as_str()],
    )?;
    Ok(barred + redact(conn, Reach::Listed(&listed), at)?.len())
}

/// One entity whose payload bytes this database has erased, with every
/// erased digest it owns: a line of the erasure manifest the first replica
/// read writes from the database (#256, B-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErasedEntity {
    /// Entity kind the feed, revision or outbox names the digest under — or
    /// the payload row's own kind when nothing names it.
    pub kind: String,
    /// Entity key — or the digest itself when nothing names it.
    pub key: String,
    /// Every erased digest the entity owns.
    pub digests: Vec<String>,
    /// When its first payload was erased.
    pub erased_at: String,
}

/// Every erased payload with the entity that owns it, oldest erasure first.
/// A row redacted before v26 has no kind and reads as erased.
const ERASED: &str = "
    WITH owner(digest, kind, key) AS (
        SELECT payload_digest, entity_kind, entity_key FROM replica_feed
         WHERE payload_digest IS NOT NULL
        UNION
        SELECT payload_digest, entity_kind, entity_key FROM replica_revision
         WHERE payload_digest IS NOT NULL
        UNION
        SELECT payload_digest, entity_kind, entity_key FROM replica_operation
         WHERE payload_digest IS NOT NULL)
    SELECT COALESCE(o.kind, p.entity_kind), COALESCE(o.key, p.digest), p.digest, p.redacted_at
      FROM replica_payload p LEFT JOIN owner o ON o.digest = p.digest
     WHERE p.redacted_at IS NOT NULL AND COALESCE(p.redaction, 'erased') = 'erased'
     ORDER BY p.redacted_at, 1, 2, p.digest";

/// Every entity whose payload bytes are erased, grouped, in the order its
/// first digest was erased.
///
/// # Errors
/// Propagates SQLite failures.
pub fn erased_entities(conn: &Connection) -> Result<Vec<ErasedEntity>> {
    let mut statement = conn.prepare(ERASED)?;
    let rows = statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<Vec<(String, String, String, String)>>>()?;
    let mut entities: Vec<ErasedEntity> = Vec::new();
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    for (kind, key, digest, erased_at) in rows {
        let owner = (kind, key);
        if let Some(&at) = seen.get(&owner) {
            entities[at].digests.push(digest);
            continue;
        }
        seen.insert(owner.clone(), entities.len());
        let (kind, key) = owner;
        entities.push(ErasedEntity {
            kind,
            key,
            digests: vec![digest],
            erased_at,
        });
    }
    Ok(entities)
}

#[cfg(test)]
#[path = "tests/replica_redaction.rs"]
mod tests;
