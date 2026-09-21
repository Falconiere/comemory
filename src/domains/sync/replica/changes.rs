//! `GET /sync/replica/changes` — the ordered page above a peer's cursor.
//!
//! Each entry carries the payload that was accepted at that position, read
//! from the immutable payload row rather than from live state, so history
//! describes what happened instead of what is true now.

use crate::domains::sync::replica::bootstrap;
use crate::domains::sync::replica::contract::{CursorRef, PROTOCOL};
use crate::domains::sync::replica::contract_views::{ChangeEntry, ChangesResponse, PayloadState};
use crate::domains::sync::replica::validate;
use crate::prelude::*;
use crate::store::replica_journal::stream_epoch;
use crate::store::replica_read::{self, FeedRow};
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

    let conn = ctx.conn()?;
    let head_sequence = replica_read::head(conn)?;
    validate::check_position(since, head_sequence)?;
    let rows = replica_read::page(conn, since, limit.clamp(MIN_LIMIT, MAX_LIMIT), kind)?;
    let next_sequence = rows.last().map(|row| row.sequence);
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
    let payload_state = if row.payload_erased {
        PayloadState::Erased
    } else if row.payload.is_some() {
        PayloadState::Present
    } else {
        PayloadState::Absent
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
