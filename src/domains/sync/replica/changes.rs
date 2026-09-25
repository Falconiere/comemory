//! `GET /sync/replica/changes` — the ordered page above a peer's cursor.
//!
//! Each entry carries the payload that was accepted at that position, read
//! from the immutable payload row rather than from live state, so history
//! describes what happened instead of what is true now.

use crate::domains::sync::replica::contract::{CursorRef, PROTOCOL};
use crate::domains::sync::replica::contract_views::{ChangeEntry, ChangesResponse, PayloadState};
use crate::domains::sync::replica::validate;
use crate::domains::sync::replica::{bootstrap, event_capture};
use crate::prelude::*;
use crate::store::replica_journal::stream_epoch;
use crate::store::replica_read::{self, FeedRow, Redaction};
use crate::utilities::context::Ctx;

/// Smallest and largest page a peer may ask for.
const MIN_LIMIT: usize = 1;
const MAX_LIMIT: usize = 500;

/// Serve one page above `since`.
///
/// # Errors
/// Returns [`Error::EpochMismatch`] when the cursor belongs to another
/// stream, [`Error::Conflict`] when it points past the head, and propagates
/// SQLite failures.
pub fn run(
    ctx: &mut Ctx<'_>,
    since: i64,
    limit: usize,
    kind: Option<&str>,
    epoch: Option<&str>,
) -> Result<ChangesResponse> {
    let stream = {
        let conn = ctx.conn()?;
        stream_epoch(conn)?
    };
    let cursor = epoch.map(|epoch| CursorRef {
        stream_epoch: epoch.to_string(),
        sequence: since,
    });
    validate::check_cursor(cursor.as_ref(), &stream)?;
    bootstrap::advance(ctx)?;
    event_capture::advance(ctx)?;

    let conn = ctx.conn()?;
    let head_sequence = replica_read::head(conn)?;
    validate::check_position(since, head_sequence)?;
    // The continuation is the last RAW position scanned, not the last match:
    // a kind-filtered page with no match still moves the reader forward.
    let (rows, next_sequence) =
        replica_read::scan(conn, since, limit.clamp(MIN_LIMIT, MAX_LIMIT), kind)?;
    let entries = rows.into_iter().map(entry).collect::<Result<Vec<_>>>()?;
    Ok(ChangesResponse {
        protocol: PROTOCOL.to_string(),
        stream_epoch: stream,
        head_sequence,
        next_sequence,
        entries,
    })
}

/// Render one stored position on the wire.
fn entry(row: FeedRow) -> Result<ChangeEntry> {
    let payload_state = match (row.redaction, row.payload.is_some()) {
        (Some(Redaction::Erased), _) => PayloadState::Erased,
        (Some(Redaction::Expired), _) => PayloadState::Expired,
        (None, true) => PayloadState::Present,
        (None, false) => PayloadState::Absent,
    };
    let payload = row
        .payload
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| Error::Other(format!("stored payload is not json: {e}")))?;
    Ok(ChangeEntry {
        sequence: row.sequence,
        operation_id: row.operation_id,
        entity_kind: row.entity_kind,
        entity_key: row.entity_key,
        op: row.op,
        schema_version: row.schema_version,
        payload_digest: row.payload_digest,
        payload,
        payload_state,
        repository: row.repository,
        at: row.at,
    })
}

#[cfg(test)]
#[path = "tests/changes.rs"]
mod tests;
