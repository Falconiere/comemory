//! `GET /sync/replica/events` — the notification-only feed.
//!
//! Frames carry a position and an entity kind and nothing else: a peer learns
//! that something changed and pulls it through `changes`, so no content
//! crosses a channel that exists only to say "look again". The feed is
//! resumable from any position, because it is read from the journal rather
//! than buffered.

use serde::{Deserialize, Serialize};

use crate::prelude::*;
use crate::store::replica_read;
use crate::utilities::context::Ctx;

/// One notification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventFrame {
    /// Position that was accepted.
    pub sequence: i64,
    /// Kind of the entity that changed — enough to skip a pull a peer does
    /// not care about, and not content.
    pub entity_kind: String,
}

/// Frames above `since`, ascending, capped at `limit`.
///
/// # Errors
/// Returns [`Error::EpochMismatch`] when `epoch` belongs to another stream,
/// [`Error::Conflict`] when `since` points past the head, and propagates
/// SQLite failures.
pub fn frames(
    ctx: &mut Ctx<'_>,
    since: i64,
    limit: usize,
    epoch: Option<&str>,
) -> Result<Vec<EventFrame>> {
    let conn = ctx.conn()?;
    let stream = crate::store::replica_journal::stream_epoch(conn)?;
    if let Some(epoch) = epoch
        && epoch != stream
    {
        return Err(Error::EpochMismatch(format!(
            "cursor is for stream {epoch}, this stream is {stream}"
        )));
    }
    crate::domains::sync::replica::validate::check_position(since, replica_read::head(conn)?)?;
    Ok(replica_read::page(conn, since, limit, None)?
        .into_iter()
        .map(|row| EventFrame {
            sequence: row.sequence,
            entity_kind: row.entity_kind,
        })
        .collect())
}

#[cfg(test)]
#[path = "tests/events.rs"]
mod tests;
