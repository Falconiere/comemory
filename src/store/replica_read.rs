//! `replica-v1` journal reads — the ordered feed page, the head position,
//! one entity's revision, and the digest sets the manifest buckets.
//!
//! A feed page carries the payload bytes that were accepted at that position,
//! never today's bytes: the join is on the digest the row recorded.

use rusqlite::Connection;
use toolu_orm::core::query_column::{CommonOps, NumericOps};

use super::orm;
use super::schema_replica::{
    ReplicaFeed, ReplicaPayload, ReplicaRevision, replica_feed as feed_col,
    replica_payload as payload_col, replica_revision as revision_col,
};
use crate::prelude::*;
use crate::store::replica_journal::{ReplicaOp, ReplicaOrigin};

/// One accepted position, with the payload it named.
#[derive(Debug, Clone)]
pub struct FeedRow {
    /// Server-assigned position.
    pub sequence: i64,
    /// Operation that was accepted here.
    pub operation_id: String,
    /// Entity kind.
    pub entity_kind: String,
    /// Entity key within its kind.
    pub entity_key: String,
    /// What the operation did.
    pub op: ReplicaOp,
    /// Digest of the accepted payload; `None` for a tombstone.
    pub payload_digest: Option<String>,
    /// Payload schema version.
    pub schema_version: i64,
    /// Canonical payload bytes; `None` for a tombstone or a redacted payload.
    pub payload: Option<String>,
    /// Why the payload row has no bytes, when it has none.
    pub redaction: Option<Redaction>,
    /// Local or imported.
    pub origin: ReplicaOrigin,
    /// Canonical repository, when the entity has one.
    pub repository: Option<String>,
    /// RFC3339 provenance time.
    pub at: String,
}

/// Why a payload's bytes are gone. The digest — and with it the barrier —
/// stays either way; the two differ in what a replay is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redaction {
    /// Permanently erased (a purge). A replay answers `payload_erased`.
    Erased,
    /// Past retention. A replay answers `payload_expired`.
    Expired,
}

impl Redaction {
    /// The stored `replica_payload.redaction` literal.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Erased => "erased",
            Self::Expired => "expired",
        }
    }

    /// Read a row's redaction state. A row redacted before v26 has
    /// `redacted_at` and no kind, and reads as erased — the only redaction
    /// that existed then.
    pub(super) fn of(redacted_at: Option<&str>, kind: Option<&str>) -> Option<Self> {
        redacted_at?;
        Some(match kind {
            Some("expired") => Self::Expired,
            _ => Self::Erased,
        })
    }
}

/// The current state of one entity.
#[derive(Debug, Clone)]
pub struct RevisionRow {
    /// Newest accepted position for the entity.
    pub sequence: i64,
    /// Digest of the newest accepted payload.
    pub payload_digest: Option<String>,
    /// Whether the entity is tombstoned.
    pub deleted: bool,
    /// Position of the tombstone a restore must reference.
    pub deleted_sequence: Option<i64>,
}

/// Feed rows with `sequence > since`, ascending, capped at `limit`, optionally
/// restricted to one entity kind.
///
/// # Errors
/// Propagates SQLite failures and a stored `op`/`origin` literal the schema's
/// `CHECK` should have refused.
pub fn page(
    conn: &Connection,
    since: i64,
    limit: usize,
    kind: Option<&str>,
) -> Result<Vec<FeedRow>> {
    let limit = i64::try_from(limit)
        .map_err(|_| Error::Other(format!("page limit not representable: {limit}")))?;
    let mut query = ReplicaFeed::select()
        .columns_typed(&[])
        .column_expr(&feed_col::sequence.qualified(), "sequence")
        .column_expr(&feed_col::operation_id.qualified(), "operation_id")
        .column_expr(&feed_col::entity_kind.qualified(), "entity_kind")
        .column_expr(&feed_col::entity_key.qualified(), "entity_key")
        .column_expr(&feed_col::op.qualified(), "op")
        .column_expr(&feed_col::payload_digest.qualified(), "payload_digest")
        .column_expr(&feed_col::schema_version.qualified(), "schema_version")
        .column_expr(&payload_col::bytes.qualified(), "payload")
        .column_expr(&payload_col::redacted_at.qualified(), "redacted_at")
        .column_expr(&payload_col::redaction.qualified(), "redaction")
        .column_expr(&feed_col::origin.qualified(), "origin")
        .column_expr(&feed_col::repository.qualified(), "repository")
        .column_expr(&feed_col::at.qualified(), "at")
        .left_join(
            "replica_payload",
            payload_col::digest.equals(&feed_col::payload_digest),
        )
        .filter(feed_col::sequence.gt(since))
        .order_by(feed_col::sequence.asc())
        .limit(limit);
    if let Some(kind) = kind {
        query = query.filter(feed_col::entity_kind.eq(kind));
    }
    let rows = orm::query_all(conn, query.to_sql(), |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Option<String>>(8)?,
            r.get::<_, Option<String>>(9)?,
            r.get::<_, String>(10)?,
            r.get::<_, Option<String>>(11)?,
            r.get::<_, String>(12)?,
        ))
    })?;
    rows.into_iter().map(decode_feed_row).collect()
}

/// A filtered page with its raw continuation: `limit` RAW positions above
/// `since` are scanned, the ones of `kind` (every one, with `None`) are
/// returned, and the last position scanned is the continuation — so a window
/// with no match still advances the reader, the way a policy-filtered
/// platform page does.
///
/// # Errors
/// Propagates SQLite failures.
pub fn scan(
    conn: &Connection,
    since: i64,
    limit: usize,
    kind: Option<&str>,
) -> Result<(Vec<FeedRow>, Option<i64>)> {
    let raw = page(conn, since, limit, None)?;
    let scanned_through = raw.last().map(|row| row.sequence);
    let rows = raw
        .into_iter()
        .filter(|row| kind.is_none_or(|k| row.entity_kind == k))
        .collect();
    Ok((rows, scanned_through))
}

/// The feed position an operation was accepted at, if it was.
///
/// Reconciliation asks this before journalling a write it is finishing: a
/// mirror row that landed proves nothing about the operation, because the
/// edit and restore paths commit the two in separate transactions. Keyed on
/// `operation_id`, which `uq_replica_feed_operation` makes unique.
///
/// # Errors
/// Propagates SQLite failures.
pub fn position_of(conn: &Connection, operation_id: &str) -> Result<Option<i64>> {
    orm::query_optional(
        conn,
        ReplicaFeed::select()
            .columns_typed(&[&feed_col::sequence])
            .filter(feed_col::operation_id.eq(operation_id))
            .to_sql(),
        |r| r.get(0),
    )
}

/// Highest accepted position, or `0` when the feed is empty.
///
/// # Errors
/// Propagates SQLite failures.
pub fn head(conn: &Connection) -> Result<i64> {
    let head: Option<i64> = orm::query_one(
        conn,
        ReplicaFeed::select()
            .column_expr("MAX(sequence)", "head")
            .to_sql(),
        |r| r.get(0),
    )?;
    Ok(head.unwrap_or(0))
}

/// The current revision of one entity, if it has ever been accepted.
///
/// # Errors
/// Propagates SQLite failures.
pub fn revision(conn: &Connection, kind: &str, key: &str) -> Result<Option<RevisionRow>> {
    let row: Option<(i64, Option<String>, i64, Option<i64>)> = orm::query_optional(
        conn,
        ReplicaRevision::select()
            .columns_typed(&[
                &revision_col::sequence,
                &revision_col::payload_digest,
                &revision_col::deleted,
                &revision_col::deleted_sequence,
            ])
            .filter(revision_col::entity_kind.eq(kind))
            .filter(revision_col::entity_key.eq(key))
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    Ok(row.map(
        |(sequence, payload_digest, deleted, deleted_sequence)| RevisionRow {
            sequence,
            payload_digest,
            deleted: deleted != 0,
            deleted_sequence,
        },
    ))
}

/// The live payload digests of every entity kind the journal knows, grouped
/// by kind.
///
/// One ordered scan rather than a kind list plus a query per kind: the
/// manifest needs both halves together, and asking twice let them disagree
/// about what was live.
///
/// A kind whose entities are all tombstoned stays in the result with an EMPTY
/// digest list. Dropping it would leave a peer unable to tell "I hold nothing
/// of this kind" from "I have never heard of this kind", and the bucket
/// comparison that would surface the difference would never run.
///
/// # Errors
/// Propagates SQLite failures.
pub fn kind_digests(conn: &Connection) -> Result<Vec<(String, Vec<String>)>> {
    let rows: Vec<(String, Option<String>, i64)> = orm::query_all(
        conn,
        ReplicaRevision::select()
            .columns_typed(&[
                &revision_col::entity_kind,
                &revision_col::payload_digest,
                &revision_col::deleted,
            ])
            .order_by(revision_col::entity_kind.asc())
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    for (kind, digest, deleted) in rows {
        let live = (deleted == 0).then_some(digest).flatten();
        match grouped.last_mut() {
            Some((last, digests)) if *last == kind => digests.extend(live),
            _ => grouped.push((kind, live.into_iter().collect())),
        }
    }
    Ok(grouped)
}

/// Decode one raw feed tuple.
type RawFeedRow = (
    i64,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    String,
);

fn decode_feed_row(raw: RawFeedRow) -> Result<FeedRow> {
    let (
        sequence,
        operation_id,
        entity_kind,
        entity_key,
        op,
        payload_digest,
        schema_version,
        payload,
        redacted_at,
        redaction,
        origin,
        repository,
        at,
    ) = raw;
    Ok(FeedRow {
        sequence,
        operation_id,
        entity_kind,
        entity_key,
        op: ReplicaOp::parse(&op)?,
        payload_digest,
        schema_version,
        payload,
        redaction: Redaction::of(redacted_at.as_deref(), redaction.as_deref()),
        origin: ReplicaOrigin::parse(&origin)?,
        repository,
        at,
    })
}

/// The canonical bytes stored for `digest`, or `None` when this engine never
/// stored them or a purge or retention removed them — what a push sends, so a
/// retry is byte-identical to the first attempt.
///
/// # Errors
/// Propagates SQLite failures.
pub fn payload_bytes(conn: &Connection, digest: &str) -> Result<Option<String>> {
    let bytes: Option<Option<String>> = orm::query_optional(
        conn,
        ReplicaPayload::select()
            .columns_typed(&[&payload_col::bytes])
            .filter(payload_col::digest.eq(digest))
            .to_sql(),
        |r| r.get(0),
    )?;
    Ok(bytes.flatten())
}

#[cfg(test)]
#[path = "tests/replica_read.rs"]
mod tests;
