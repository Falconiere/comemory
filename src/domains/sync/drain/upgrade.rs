//! The first pass after a key selects `replica-v1`.
//!
//! Two things happen once. When the old protocol ever delivered anything for
//! the workspace, the key records `upgrade_through` — the upstream head at
//! the selection — so memory operations made before it wait (held `upgrade`)
//! until the pull has read that far: the old protocol may already have
//! delivered them, and the pull is what recognizes that by content. A
//! workspace whose legacy push never delivered anything has nothing to
//! recognize, so its operations flow at once.
//! And document revisions journalled before the outbox queued them (#253
//! captured revisions but queued nothing) are queued now.

use crate::domains::documents::replica_payload::DOCUMENT_ENTITY_KIND;
use crate::domains::sync::drain::adopt;
use crate::prelude::*;
use crate::store::sync_exchange::ExchangeRow;
use crate::store::{Connection, sync_state};

/// Begin the upgrade: record the horizon when a legacy delivery was
/// possible, and queue unqueued revisions.
///
/// # Errors
/// Propagates SQLite failures.
pub fn begin(conn: &Connection, row: &mut ExchangeRow, head: i64, at: &str) -> Result<usize> {
    let delivered = sync_state::get(conn, &row.key.workspace_id)?.is_some_and(|s| s.pushed_seq > 0);
    row.upgrade_through = delivered.then_some(head);
    adopt::all(conn, |kind| kind == DOCUMENT_ENTITY_KIND, at)
}

#[cfg(test)]
#[path = "tests/upgrade.rs"]
mod tests;
