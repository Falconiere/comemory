//! Queueing local feed positions the outbox never saw: document revisions
//! #253 journalled before the outbox existed (once, at the upgrade), and the
//! feedback and activity events #254 journals but never queues (before every
//! push, from one cursor per kind). Nothing reads a client's feed, which is
//! where #254 captures runs, so the drain advances one capture batch first.
//! Reads are filtered by kind, and adoption skips any operation already
//! queued, so a cursor a rebuild resets costs a rescan, never a second upload.

use crate::config::{Config, Paths};
use crate::domains::learning::replica_payload::FEEDBACK_ENTITY_KIND;
use crate::domains::sync::drain::session::Mode;
use crate::domains::sync::replica::activity_payload::ACTIVITY_ENTITY_KIND;
use crate::domains::sync::replica::event_capture;
use crate::prelude::*;
use crate::store::replica_journal::{NewOperation, PayloadRef, ReplicaOrigin};
use crate::store::replica_outbox::{self, Scope};
use crate::store::{Connection, replica_read, schema_meta};
use crate::utilities::context::Ctx;

/// Feed positions read per page while adopting.
const PAGE: usize = 500;

/// The kinds adopted before every push.
const EVENT_KINDS: [&str; 2] = [FEEDBACK_ENTITY_KIND, ACTIVITY_ENTITY_KIND];

/// `schema_meta` key prefix; `<prefix>:<kind>` holds the last feed position
/// adoption read for that kind.
pub const EVENTS_THROUGH: &str = "replica_event_adopted_through";

/// How much of the event backlog one pass may adopt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Everything above the cursors.
    All,
    /// One page per kind: the inline push after a save must not scan a
    /// backlog; the cursors carry the rest to the next pass.
    OnePage,
}

impl Reach {
    /// The reach a pass run in `mode` gets.
    #[must_use]
    pub fn of(mode: Mode) -> Self {
        if matches!(mode, Mode::Inline(_)) {
            Self::OnePage
        } else {
            Self::All
        }
    }
}

/// Advance one capture batch, then queue the local events journalled since
/// the last pass, as far as `reach` allows; returns how many were queued.
/// Each page commits with the cursor that covers it, so an interrupted pass
/// resumes where it stopped.
///
/// # Errors
/// Propagates SQLite failures from adoption. A capture that fails rolls its
/// batch back whole, is logged, and is retried by the next pass; it never
/// fails the push of what is already journalled.
pub fn events(
    (paths, cfg): (&Paths, &Config),
    conn: &mut Connection,
    at: &str,
    reach: Reach,
) -> Result<usize> {
    if let Err(e) = event_capture::advance(&mut Ctx::borrowed(paths, cfg, conn)) {
        tracing::warn!(error = %e, "event capture deferred to the next pass");
    }
    let mut adopted = 0;
    for kind in EVENT_KINDS {
        adopted += from_cursor(conn, kind, at, reach)?;
    }
    Ok(adopted)
}

/// Queue `kind`'s positions above its cursor, one transaction per page.
fn from_cursor(conn: &mut Connection, kind: &str, at: &str, reach: Reach) -> Result<usize> {
    let key = format!("{EVENTS_THROUGH}:{kind}");
    let mut since = schema_meta::get(conn, &key)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut adopted = 0;
    loop {
        let tx = conn.transaction()?;
        let Some((queued, through)) = page(&tx, since, kind, at)? else {
            return Ok(adopted);
        };
        schema_meta::upsert(&tx, &key, &through.to_string())?;
        tx.commit()?;
        adopted += queued;
        since = through;
        if reach == Reach::OnePage {
            return Ok(adopted);
        }
    }
}

/// Queue every local position of `kind`, from the start of the feed; returns
/// how many.
///
/// # Errors
/// Propagates SQLite failures.
pub fn all(conn: &Connection, kind: &str, at: &str) -> Result<usize> {
    let mut since = 0;
    let mut adopted = 0;
    while let Some((queued, through)) = page(conn, since, kind, at)? {
        adopted += queued;
        since = through;
    }
    Ok(adopted)
}

/// Queue the unqueued local positions of one page of `kind` above `since`;
/// `None` when the feed has none above it, else how many were queued and the
/// last position read.
fn page(conn: &Connection, since: i64, kind: &str, at: &str) -> Result<Option<(usize, i64)>> {
    let rows = replica_read::page(conn, since, PAGE, Some(kind))?;
    let Some(through) = rows.last().map(|row| row.sequence) else {
        return Ok(None);
    };
    let mut queued = 0;
    for row in rows {
        if row.origin != ReplicaOrigin::Local
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
