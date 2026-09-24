//! Journal the shared events that are not journalled at record time (#254):
//! every activity run, and — once — the verdicts recorded before sharing
//! existed. Advanced beside [`super::bootstrap::advance`] in bounded,
//! restartable batches. Activity is captured here, not where it is recorded,
//! because recording is best-effort by contract: a failed capture is retried
//! by the next call and never fails the command that ran.

use crate::domains::learning::feedback_share::journal_retained;
use crate::domains::learning::replica_payload::mint_event_id;
use crate::domains::sync::replica::activity_payload::{
    ACTIVITY_ENTITY_KIND, ACTIVITY_PAYLOAD_VERSION, ActivityEventV1, shared_keys,
};
use crate::domains::sync::replica::bootstrap::{Progress, STATE_COMPLETE};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::activity::ActivityRow;
use crate::store::replica_journal::{self, LocalEvent, PayloadRef};
use crate::store::{
    activity_share, feedback_share, replica_device, repository_approval, schema_meta,
};
use crate::utilities::context::Ctx;

/// Rows journalled per call, per walk.
const BATCH: usize = 200;

/// `schema_meta` key holding the last `activity_log.id` the sweep read.
pub const ACTIVITY_THROUGH: &str = "replica_activity_capture_through";

/// `schema_meta` key holding `pending`, `seeding` or `complete`.
pub const BACKFILL_STATE: &str = "replica_feedback_backfill_state";

/// `schema_meta` key holding the last `feedback_events.id` backfill read.
pub const BACKFILL_THROUGH: &str = "replica_feedback_backfill_through";

/// Advance both walks by one batch; returns the backfill's progress, which
/// gates the advertised capability as the memory bootstrap does.
///
/// # Errors
/// Propagates SQLite failures and payload serialization. A batch's
/// transaction rolls back whole, so its cursor does not move either.
pub fn advance(ctx: &mut Ctx<'_>) -> Result<Progress> {
    let conn = ctx.conn()?;
    capture_activity(conn)?;
    backfill_verdicts(conn)
}

/// Journal the next batch of shareable runs recorded here.
fn capture_activity(conn: &mut Connection) -> Result<()> {
    let through = cursor(conn, ACTIVITY_THROUGH)?;
    let rows = activity_share::capture_batch_after(conn, through, BATCH)?;
    let Some(last) = rows.last().map(|r| r.id) else {
        return Ok(());
    };
    let tx = conn.transaction()?;
    let device = replica_device::id(&tx)?;
    let mut shared = 0_usize;
    for row in &rows {
        shared += usize::from(journal_run(&tx, &device, row)?);
    }
    schema_meta::upsert(&tx, ACTIVITY_THROUGH, &last.to_string())?;
    tx.commit()?;
    // Counts only: a run's text is exactly what must not reach a log line.
    tracing::debug!(
        read = rows.len(),
        shared,
        through = last,
        "activity capture batch"
    );
    Ok(())
}

/// Journal one run when it may be shared: an allowlisted command, scoped to a
/// repository an approval resolves. Returns whether it was.
fn journal_run(tx: &Connection, device: &str, row: &ActivityRow) -> Result<bool> {
    if shared_keys(&row.command).is_none() {
        return Ok(false);
    }
    let Some(label) = row.repo.as_deref() else {
        return Ok(false);
    };
    let Some(canonical) = repository_approval::canonical_for(tx, label)? else {
        return Ok(false);
    };
    let event_id = mint_event_id()?;
    let Some(payload) = ActivityEventV1::from_row(row, &canonical, device, &event_id) else {
        return Ok(false);
    };
    let (bytes, digest) = payload.canonical()?;
    replica_journal::append_local_event(
        tx,
        &LocalEvent {
            entity_kind: ACTIVITY_ENTITY_KIND,
            event_id: &event_id,
            schema_version: ACTIVITY_PAYLOAD_VERSION,
            repository: &canonical,
            payload: PayloadRef {
                digest: &digest,
                bytes: &bytes,
            },
            at: &row.at,
        },
    )?;
    activity_share::stamp_event_id(tx, row.id, &event_id)?;
    Ok(true)
}

/// Journal the next batch of retained verdicts recorded before sharing
/// existed. One pass, then `complete` for good: from then on record-time
/// capture is the only path. Counters are never touched — this machine's
/// already include every verdict, retained or expired.
fn backfill_verdicts(conn: &mut Connection) -> Result<Progress> {
    let state = schema_meta::get(conn, BACKFILL_STATE)?.unwrap_or_else(|| "pending".to_string());
    let through = cursor(conn, BACKFILL_THROUGH)?;
    if state == STATE_COMPLETE {
        return Ok(Progress {
            state,
            through: through.to_string(),
        });
    }
    let rows = feedback_share::unshared_legacy_after(conn, through, BATCH)?;
    let last = rows.last().map_or(through, |r| r.id);
    let finished = rows.len() < BATCH;
    let tx = conn.transaction()?;
    for row in &rows {
        journal_retained(&tx, row)?;
    }
    let state = if finished { STATE_COMPLETE } else { "seeding" };
    schema_meta::upsert(&tx, BACKFILL_STATE, state)?;
    schema_meta::upsert(&tx, BACKFILL_THROUGH, &last.to_string())?;
    tx.commit()?;
    Ok(Progress {
        state: state.to_string(),
        through: last.to_string(),
    })
}

/// A stored cursor, or `0` before the first batch.
fn cursor(conn: &Connection, key: &str) -> Result<i64> {
    Ok(schema_meta::get(conn, key)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

#[cfg(test)]
#[path = "tests/event_capture.rs"]
mod tests;
