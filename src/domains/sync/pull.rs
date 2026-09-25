//! One legacy pull page: platform sync entries above `pulled_seq`, applied
//! through `domains::sync::exchange::import`.
//!
//! The cursor never passes an entry the import failed to handle. `accepted`,
//! `exists`, `stale` and `deleted` advance it; `secret_detected`,
//! `repo_not_allowed` and `id_collision` have no local remedy, so they advance
//! it as holds counted under the `legacy` epoch; `invalid`, or an import that
//! fails outright, stalls the pull before the entry. The drain loop calls this
//! page after page — there is no per-run cap.

use crate::config::{Config, Paths};
use crate::domains::sync::drain::transport::{Answer, Transport};
use crate::domains::sync::exchange::{
    ChangesResponse, ImportEntry, ImportItemResult, ImportRequest, ImportStatus, SyncEntry,
};
use crate::prelude::*;
use crate::store::replica_pull_hold::{self, LEGACY_EPOCH, PullHold};
use crate::store::sync_exchange::ExchangeKey;
use crate::store::{Connection, sync_state};
use crate::utilities::context::Ctx;

/// Entries asked for per page.
const MAX_BATCH: usize = 500;

/// Outcome counters for a legacy pull.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PullStats {
    /// Entries applied from the remote log.
    pub pulled: u32,
    /// Entries passed as holds (secret, policy, id collision).
    pub held: u32,
    /// Highest server seq handled locally.
    pub last_pulled_seq: i64,
    /// The entry the pull stalled before, and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stalled: Option<(i64, String)>,
}

/// What one page did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Paged {
    /// Whether the cursor moved.
    pub advanced: bool,
    /// Whether the cursor reached the head the page reported.
    pub done: bool,
}

/// Pull and import one page above the key's `pulled_seq`.
///
/// # Errors
/// Propagates SQLite failures; the transport failure is the inner `Err`.
pub fn page(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    (wire, key): (&Transport, &ExchangeKey),
    stats: &mut PullStats,
    at: &str,
) -> Result<Answer<Paged>> {
    let workspace = key.workspace_id.as_str();
    sync_state::ensure(conn, workspace, &key.api_url)?;
    let since = sync_state::get(conn, workspace)?
        .ok_or_else(|| Error::Other("sync_state missing after ensure".into()))?
        .pulled_seq;
    let query = [
        ("since", since.to_string()),
        ("limit", MAX_BATCH.to_string()),
    ];
    let remote = match wire.retrying(|t| t.get::<ChangesResponse>("/v1/sync/changes", &query)) {
        Ok(remote) => remote,
        Err(failure) => return Ok(Err(failure)),
    };
    let mut cursor = since;
    if !remote.entries.is_empty() {
        cursor = walk(paths, cfg, conn, (key, since), &remote.entries, stats, at)?;
    }
    if stats.stalled.is_none()
        && let Some(next) = remote.next_seq.filter(|next| *next > cursor)
    {
        cursor = next;
    }
    if cursor > since {
        sync_state::set_pulled(conn, workspace, cursor, at)?;
    }
    stats.last_pulled_seq = cursor;
    Ok(Ok(Paged {
        advanced: cursor > since,
        done: stats.stalled.is_some() || remote.next_seq.is_none() || cursor >= remote.head_seq,
    }))
}

/// Import the page, then walk its answers in order; the cursor it reached.
fn walk(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    (key, since): (&ExchangeKey, i64),
    entries: &[SyncEntry],
    stats: &mut PullStats,
    at: &str,
) -> Result<i64> {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    let results = import(&mut ctx, since, entries, stats);
    let mut cursor = since;
    for (entry, result) in entries.iter().zip(&results) {
        match handled(result) {
            Handled::Advanced { applied } => stats.pulled += u32::from(applied),
            Handled::Held(reason) => {
                hold(conn, key, entry, reason, at)?;
                stats.held += 1;
            }
            Handled::Stalled(why) => {
                stats.stalled = Some((entry.seq, why));
                return Ok(cursor);
            }
        }
        cursor = entry.seq;
    }
    if stats.stalled.is_none() && results.len() < entries.len() {
        let missing = entries[results.len()].seq;
        stats.stalled = Some((
            missing,
            "the import gave no answer for this entry".to_string(),
        ));
    }
    Ok(cursor)
}

/// The page's import answers. The import commits entry by entry and a hard
/// failure (I/O, SQLite) drops every answer, so the page is then imported one
/// entry at a time — the committed ones answer `exists` — to learn which
/// entry failed: the answers before it are returned and the stall names it.
fn import(
    ctx: &mut Ctx<'_>,
    since: i64,
    entries: &[SyncEntry],
    stats: &mut PullStats,
) -> Vec<ImportItemResult> {
    let whole = ImportRequest {
        cursor: since,
        entries: entries.iter().map(import_entry).collect(),
    };
    if let Ok(response) = crate::domains::sync::exchange::import::run(ctx, whole, None) {
        return response.results;
    }
    let mut results = Vec::with_capacity(entries.len());
    for entry in entries {
        let one = ImportRequest {
            cursor: since,
            entries: vec![import_entry(entry)],
        };
        match crate::domains::sync::exchange::import::run(ctx, one, None) {
            Ok(response) => results.extend(response.results),
            Err(e) => {
                stats.stalled = Some((entry.seq, e.to_string()));
                break;
            }
        }
    }
    results
}

/// What one import answer does to the cursor.
enum Handled {
    /// Handled here (applied, or already so); the cursor passes it.
    Advanced {
        /// Whether this import applied it.
        applied: bool,
    },
    /// A refusal with no local remedy; passed as a hold with this reason.
    Held(&'static str),
    /// Not handled; the pull stops before it, for this reason.
    Stalled(String),
}

/// Classify one import answer.
fn handled(result: &ImportItemResult) -> Handled {
    match result.status {
        ImportStatus::Accepted => Handled::Advanced { applied: true },
        ImportStatus::Exists | ImportStatus::Stale | ImportStatus::Deleted => {
            Handled::Advanced { applied: false }
        }
        ImportStatus::SecretDetected => Handled::Held("secret"),
        ImportStatus::RepoNotAllowed => Handled::Held("policy"),
        ImportStatus::IdCollision => Handled::Held("id_collision"),
        ImportStatus::Invalid => Handled::Stalled(
            result
                .reason
                .clone()
                .unwrap_or_else(|| "invalid".to_string()),
        ),
    }
}

/// The import body for one pulled entry.
fn import_entry(entry: &SyncEntry) -> ImportEntry {
    ImportEntry {
        op: entry.op,
        id: entry.id.clone(),
        content_hash: entry.content_hash.clone(),
        at: entry.at.clone(),
        record: entry.record.clone(),
    }
}

/// Record a legacy hold for one passed entry.
fn hold(
    conn: &Connection,
    key: &ExchangeKey,
    entry: &SyncEntry,
    reason: &str,
    at: &str,
) -> Result<()> {
    let hold = PullHold {
        from_sequence: entry.seq,
        to_sequence: entry.seq,
        reason: reason.to_string(),
        entity_kind: Some("memory".to_string()),
        entity_key: Some(entry.id.clone()),
        repository: entry.record.as_ref().map(|r| r.frontmatter.repo.clone()),
        policy_revision: None,
    };
    replica_pull_hold::record(conn, key, LEGACY_EPOCH, &hold, at)
}

#[cfg(test)]
#[path = "tests/pull.rs"]
mod tests;
