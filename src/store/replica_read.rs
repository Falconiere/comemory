//! `replica-v1` journal reads — the ordered feed page, the head position,
//! one entity's revision, and the digest sets the manifest buckets.
//!
//! A feed page carries the payload bytes that were accepted at that position,
//! never today's bytes: the join is on the digest the row recorded.

use rusqlite::Connection;
use toolu_orm::core::query_column::{CommonOps, NumericOps};

use super::schema_replica::{
    ReplicaFeed, ReplicaRevision, replica_feed as feed_col, replica_payload as payload_col,
    replica_revision as revision_col,
};
use super::{orm, schema_replica};
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
    /// Canonical payload bytes; `None` for a tombstone or an erased payload.
    pub payload: Option<String>,
    /// `true` when the payload row exists but its bytes were erased.
    pub payload_erased: bool,
    /// Local or imported.
    pub origin: ReplicaOrigin,
    /// Canonical repository, when the entity has one.
    pub repository: Option<String>,
    /// RFC3339 provenance time.
    pub at: String,
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
            r.get::<_, String>(9)?,
            r.get::<_, Option<String>>(10)?,
            r.get::<_, String>(11)?,
        ))
    })?;
    rows.into_iter().map(decode_feed_row).collect()
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

/// Whether retention has erased the bytes behind `digest`.
///
/// A narrow probe rather than a row read: acceptance only needs to know
/// whether the barrier stands, and a feed page already carries the bytes.
///
/// # Errors
/// Propagates SQLite failures.
pub fn is_erased(conn: &Connection, digest: &str) -> Result<bool> {
    let erased: i64 = orm::query_one(
        conn,
        schema_replica::ReplicaPayload::select()
            .filter(payload_col::digest.eq(digest))
            .filter(payload_col::redacted_at.is_not_null())
            .to_count_sql(),
        |r| r.get(0),
    )?;
    Ok(erased > 0)
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
        payload_erased: redacted_at.is_some(),
        origin: ReplicaOrigin::parse(&origin)?,
        repository,
        at,
    })
}

#[cfg(test)]
#[path = "tests/replica_read.rs"]
mod tests;
