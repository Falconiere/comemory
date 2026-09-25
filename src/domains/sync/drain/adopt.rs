//! Queueing local feed positions the outbox never saw.
//!
//! Two kinds of local position reach the feed without an outbox row. Document
//! revisions #253 journalled before the outbox queued anything are adopted
//! once, when a key upgrades ([`super::upgrade`]). Feedback and activity
//! events are journalled and never queued (#254): their feed position is the
//! durable record, and the drain reads it here before every push, from a
//! cursor, so each pass only reads what was journalled since the last one.
//! A run reaches the feed only when #254's capture batch runs, which the
//! engine does when a peer reads its feed — and nothing reads a client's, so
//! the drain advances one capture batch first.
//!
//! Adoption is idempotent — a position whose operation is already queued is
//! skipped — so a cursor a rebuild resets costs one rescan, never a second
//! upload.

use crate::domains::sync::replica::event_capture;
use crate::domains::sync::replica::validate_events::is_event_kind;
use crate::prelude::*;
use crate::store::replica_journal::{NewOperation, PayloadRef, ReplicaOrigin};
use crate::store::replica_outbox::{self, Scope};
use crate::store::{Connection, replica_read, schema_meta};
use crate::utilities::context::Ctx;

/// Feed positions read per page while adopting.
const PAGE: usize = 500;

/// `schema_meta` key holding the last feed position event adoption read.
pub const EVENTS_THROUGH: &str = "replica_event_adopted_through";

/// Advance one capture batch, then queue every local event journalled since
/// the last pass; returns how many were queued. Each page commits with the
/// cursor that covers it, so an interrupted pass resumes where it stopped.
///
/// # Errors
/// Propagates SQLite failures from adoption. A capture that fails rolls its
/// batch back whole, is logged, and is retried by the next pass; it never
/// fails the push of what is already journalled.
pub fn events(ctx: &mut Ctx<'_>, at: &str) -> Result<usize> {
    if let Err(e) = event_capture::advance(ctx) {
        tracing::warn!(error = %e, "event capture deferred to the next pass");
    }
    let conn = ctx.conn()?;
    let mut since = schema_meta::get(conn, EVENTS_THROUGH)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut adopted = 0;
    loop {
        let tx = conn.transaction()?;
        let Some((queued, through)) = page(&tx, since, is_event_kind, at)? else {
            return Ok(adopted);
        };
        schema_meta::upsert(&tx, EVENTS_THROUGH, &through.to_string())?;
        tx.commit()?;
        adopted += queued;
        since = through;
    }
}

/// Queue every local position of a kind `wanted` accepts, from the start of
/// the feed; returns how many.
///
/// # Errors
/// Propagates SQLite failures.
pub fn all(conn: &Connection, wanted: fn(&str) -> bool, at: &str) -> Result<usize> {
    let mut since = 0;
    let mut adopted = 0;
    while let Some((queued, through)) = page(conn, since, wanted, at)? {
        adopted += queued;
        since = through;
    }
    Ok(adopted)
}

/// Queue the unqueued local positions of one page above `since`; `None` when
/// the feed has nothing above it, else how many were queued and the last
/// position read.
fn page(
    conn: &Connection,
    since: i64,
    wanted: fn(&str) -> bool,
    at: &str,
) -> Result<Option<(usize, i64)>> {
    let rows = replica_read::page(conn, since, PAGE, None)?;
    let Some(through) = rows.last().map(|row| row.sequence) else {
        return Ok(None);
    };
    let mut queued = 0;
    for row in rows {
        if row.origin != ReplicaOrigin::Local
            || !wanted(&row.entity_kind)
            || !replica_outbox::read(conn, Scope::Operation(&row.operation_id), 1)?.is_empty()
        {
            continue;
        }
        let payload = row
            .payload_digest
            .as_deref()
            .map(|digest| PayloadRef { digest, bytes: "" });
        replica_outbox::enqueue(
            conn,
            &NewOperation {
                operation_id: &row.operation_id,
                entity_kind: &row.entity_kind,
                entity_key: &row.entity_key,
                op: row.op,
                payload,
                schema_version: row.schema_version,
                repository: row.repository.as_deref(),
                origin: ReplicaOrigin::Local,
                at,
            },
            None,
        )?;
        queued += 1;
    }
    Ok(Some((queued, through)))
}

#[cfg(test)]
#[path = "tests/adopt.rs"]
mod tests;
