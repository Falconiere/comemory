//! Phase 1 of a compacting replay: read the upstream from position 0 to the
//! replay target and keep, per entity, only its LAST entry.
//!
//! Nothing but outbox rows is written here — rules 1 and 5 run at every
//! position because they only settle this client's own operations (one
//! delivered at an earlier position is still settled when a later entry is
//! the entity's last). The scan resumes from `replay_scan_through` after a
//! kill, a budget end or a network failure.

use crate::domains::sync::drain::pull::Step;
use crate::domains::sync::drain::pull_rules::{self, Rule};
use crate::domains::sync::drain::transport::Failure;
use crate::domains::sync::replica::contract_views::{ChangeEntry, ChangesResponse};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_outbox::{self, Outcome};
use crate::store::replica_replay::{self, ReplayRow};
use crate::store::sync_exchange::ExchangeRow;

/// How a scan page ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scanned {
    /// More pages to read.
    More,
    /// The target is reached; the apply phase may begin.
    Done,
    /// The request failed.
    Failed(Failure),
}

/// Read one page of the replay; `kind` narrows a repair to one entity kind.
///
/// # Errors
/// Propagates SQLite and JSON failures.
pub fn page(
    conn: &Connection,
    step: &Step<'_>,
    row: &mut ExchangeRow,
    epoch: &str,
    kind: Option<&str>,
) -> Result<Scanned> {
    let since = row.replay_scan_through.unwrap_or(0);
    let target = row.replay_target.unwrap_or(0);
    if since >= target {
        return Ok(Scanned::Done);
    }
    let mut query = vec![
        ("since", since.to_string()),
        ("limit", super::pull::PAGE.to_string()),
        ("epoch", epoch.to_string()),
    ];
    if let Some(kind) = kind {
        query.push(("kind", kind.to_string()));
    }
    let page = match step
        .pull
        .transport
        .retrying(|t| t.get::<ChangesResponse>("/v1/sync/replica/changes", &query))
    {
        Ok(page) => page,
        Err(failure) => return Ok(Scanned::Failed(failure)),
    };
    for entry in page.entries.iter().filter(|e| e.sequence <= target) {
        settle(conn, step, epoch, entry)?;
        replica_replay::offer(
            conn,
            step.pull.key,
            &ReplayRow {
                entity_kind: entry.entity_kind.clone(),
                entity_key: entry.entity_key.clone(),
                sequence: entry.sequence,
                entry_json: serde_json::to_string(entry)?,
            },
        )?;
    }
    let through = page.next_sequence.unwrap_or(target).min(target);
    row.replay_scan_through = Some(through);
    Ok(if through >= target || page.next_sequence.is_none() {
        Scanned::Done
    } else {
        Scanned::More
    })
}

/// Rules 1 and 5 at one scanned position: settle this client's own
/// operations the upstream already holds.
fn settle(conn: &Connection, step: &Step<'_>, epoch: &str, entry: &ChangeEntry) -> Result<()> {
    // The same dispositions the forward pull records: an acknowledgement
    // arriving by pull, or the same bytes delivered under another id.
    let (settled, disposition) =
        match pull_rules::classify(conn, step.pull.key, step.pull.policy, epoch, entry)? {
            Rule::Own => (vec![entry.operation_id.clone()], "accepted"),
            Rule::AlreadyUpstream(ids) => (ids, "already_upstream"),
            _ => return Ok(()),
        };
    for id in &settled {
        let accepted = Outcome::Accepted {
            sequence: Some(entry.sequence),
            disposition,
            epoch: Some(epoch),
        };
        replica_outbox::record(conn, id, accepted, step.at)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/replay_scan.rs"]
mod tests;
