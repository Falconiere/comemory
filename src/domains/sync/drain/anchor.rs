//! Whether the upstream still holds, at the cursor's position, the entry the
//! cursor was left on — the one stream rewrite an epoch cannot reveal (a
//! restore that kept its epoch and has already written past the cursor).

use crate::domains::sync::drain::transport::{Failure, Transport};
use crate::domains::sync::replica::contract_views::ChangesResponse;
use crate::store::replica_cursor::Cursor;

/// What the upstream holds at the cursor's position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// The same entry: the stream the cursor was taken on is intact.
    Intact,
    /// The same position names another operation: the stream was rewritten.
    Rewritten,
    /// Nothing to compare (no anchor, or the position is not an entry this
    /// client sees).
    Unknown,
}

/// Re-read the cursor's position and compare it with the anchor.
///
/// # Errors
/// The transport failure, for the pass to record.
pub fn check(transport: &Transport, cursor: &Cursor) -> Result<Check, Failure> {
    let Some(anchor) = cursor
        .anchor
        .as_ref()
        .filter(|a| a.sequence == cursor.applied_sequence)
    else {
        return Ok(Check::Unknown);
    };
    let query = [
        ("since", (anchor.sequence - 1).to_string()),
        ("limit", "1".to_string()),
        ("epoch", cursor.stream_epoch.clone()),
    ];
    let page =
        transport.retrying(|t| t.get::<ChangesResponse>("/v1/sync/replica/changes", &query))?;
    Ok(match page.entries.first() {
        Some(entry)
            if entry.sequence == anchor.sequence && entry.operation_id == anchor.operation_id =>
        {
            Check::Intact
        }
        Some(entry) if entry.sequence == anchor.sequence => Check::Rewritten,
        _ => Check::Unknown,
    })
}

#[cfg(test)]
#[path = "tests/anchor.rs"]
mod tests;
