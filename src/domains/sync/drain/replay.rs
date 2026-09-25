//! One step of a replay in progress — a rebootstrap or a verify repair — in
//! the phase it stopped in: a scan page, or an apply batch.

use crate::domains::sync::drain::pull::Step;
use crate::domains::sync::drain::rebootstrap;
use crate::domains::sync::drain::replay_apply::{self, Applied};
use crate::domains::sync::drain::replay_scan::{self, Scanned};
use crate::domains::sync::drain::transport::Failure;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_cursor::{self, Cursor};
use crate::store::sync_exchange::ExchangeRow;

/// What one replay step did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replayed {
    /// It moved; this many entries were applied.
    Moved(u32),
    /// The apply phase stalled before this position; the replay stays.
    Stalled(i64, String),
    /// The upstream answered as a replaced stream.
    Replaced,
    /// The request failed.
    Failed(Failure),
}

/// Advance the replay `row` records by one step. A finished rebootstrap
/// moves the cursor to the replay target; a repair never moves it.
///
/// # Errors
/// Propagates SQLite and JSON failures.
pub fn step(
    conn: &mut Connection,
    step: &Step<'_>,
    row: &mut ExchangeRow,
    cursor: &mut Cursor,
) -> Result<Replayed> {
    let epoch = cursor.stream_epoch.clone();
    let kind = row.replay_kind.clone().unwrap_or_default();
    if row.replay_state.as_deref() == Some("scanning") {
        return Ok(
            match replay_scan::page(conn, step, row, &epoch, kind.strip_prefix("repair:"))? {
                Scanned::Failed(failure) if failure.replaced_stream() => Replayed::Replaced,
                Scanned::Failed(failure) => Replayed::Failed(failure),
                Scanned::Done => {
                    row.replay_state = Some("applying".to_string());
                    Replayed::Moved(0)
                }
                Scanned::More => Replayed::Moved(0),
            },
        );
    }
    Ok(match replay_apply::batch(conn, step, &epoch)? {
        Applied::More(n) => Replayed::Moved(n),
        Applied::Stalled(sequence, reason) => Replayed::Stalled(sequence, reason),
        Applied::Done(n) => {
            if kind == rebootstrap::REBOOTSTRAP {
                cursor.applied_sequence = row.replay_target.unwrap_or(0);
                cursor.anchor = None;
                replica_cursor::save(conn, cursor, step.at)?;
            }
            rebootstrap::finish(row);
            row.stall_sequence = None;
            row.stall_reason = None;
            Replayed::Moved(n)
        }
    })
}
