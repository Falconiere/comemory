//! Push local sync-log entries to the platform.
//!
//! Each entry must resolve to the platform's approved canonical GitHub
//! identity through its exact canonical label, an administrator-confirmed
//! mapping, or an unambiguous indexed checkout. The wire binds each memory id
//! to that identity without changing local frontmatter. `skip_repos` and the
//! secret scanner can withhold entries further.

use std::collections::BTreeMap;

use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::config::Paths;
use crate::domains::memories::MemoryStore;
use crate::domains::sync::drain::transport::{Answer, Transport};
use crate::domains::sync::exchange::changes::enrich_record;
use crate::domains::sync::exchange::{ImportEntry, ImportResponse, ImportStatus, SyncOp};
use crate::domains::sync::redact;
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::domains::sync::skip_repos::SkipMatcher;
use crate::prelude::*;
use crate::store::sync_exchange::ExchangeKey;
use crate::store::{Connection, memory_repository, sync_binding, sync_log, sync_state};

const MAX_BATCH: usize = 500;

/// Counters surfaced by `comemory sync --status` and returned from push.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PushStats {
    /// Entries accepted by the platform in this run.
    pub pushed: u32,
    /// Skipped — label matched `[sync] skip_repos`.
    pub skipped_config: u32,
    /// Withheld because no approved canonical GitHub identity could be
    /// established for the memory's local repository label.
    pub blocked_repo: u32,
    /// Blocked — secret rule hit without override.
    pub blocked_secrets: u32,
    /// Rejected by the platform allowlist gate (`repo_not_allowed`). A reject
    /// stops the push before its cursor advances.
    pub rejected_repo: u32,
    /// Highest local seq included in a successful batch.
    pub last_pushed_seq: i64,
}

/// Everything a legacy push batch sends through.
pub struct Wire<'a> {
    /// Legacy calls (managed when the origin is).
    pub transport: &'a Transport,
    /// The session key.
    pub key: &'a ExchangeKey,
    /// The session's policy.
    pub policy: &'a RepositoryPolicy,
    /// `[sync] skip_repos`.
    pub skip: &'a SkipMatcher,
}

/// Push one batch of local-origin log entries above `pushed_seq`; whether
/// the cursor moved (`false` once the log is drained).
///
/// # Errors
/// Propagates store and markdown failures, and a batch the upstream refused
/// under repository policy; the transport failure is the inner `Err`.
pub fn batch(
    paths: &Paths,
    conn: &mut Connection,
    wire: &Wire<'_>,
    stats: &mut PushStats,
) -> Result<Answer<bool>> {
    let workspace_id = wire.key.workspace_id.as_str();
    sync_state::ensure(conn, workspace_id, &wire.key.api_url)?;
    let row = sync_state::get(conn, workspace_id)?
        .ok_or_else(|| Error::Other("sync_state missing after ensure".into()))?;
    let rows = sync_log::local_entries_since(conn, row.pushed_seq, MAX_BATCH)?;
    let Some(high) = rows.last().map(|r| r.seq) else {
        return Ok(Ok(false));
    };
    let store = MemoryStore::new(paths.clone());
    let mut entries: Vec<ImportEntry> = Vec::new();
    let mut repositories = BTreeMap::new();
    for log_row in rows {
        if let Some((entry, repository)) = build_import_entry(&store, conn, &log_row, wire, stats)?
        {
            repositories.insert(entry.id.clone(), repository);
            entries.push(entry);
        }
    }
    if !entries.is_empty() {
        let body = WireImportRequest {
            cursor: row.pulled_seq,
            entries: &entries,
            repositories: wire.transport.is_managed().then_some(&repositories),
        };
        let sent = wire
            .transport
            .retrying(|t| t.post::<ImportResponse, _>("/v1/sync/import", &body));
        match sent {
            Ok(response) => apply_response(&response, high, stats)?,
            Err(failure) => return Ok(Err(failure)),
        }
    }
    stats.last_pushed_seq = high;
    sync_state::set_pushed(conn, workspace_id, high, &now_iso()?)?;
    Ok(Ok(true))
}

/// The import body. The managed wire adds the canonical repository each
/// memory id is bound to; an unmanaged engine's import denies unknown fields,
/// so it never sees one.
#[derive(serde::Serialize)]
struct WireImportRequest<'a> {
    cursor: i64,
    entries: &'a [ImportEntry],
    #[serde(skip_serializing_if = "Option::is_none")]
    repositories: Option<&'a BTreeMap<String, String>>,
}

fn apply_response(resp: &ImportResponse, batch_high_seq: i64, stats: &mut PushStats) -> Result<()> {
    let accepted = resp
        .results
        .iter()
        .filter(|result| {
            matches!(
                result.status,
                ImportStatus::Accepted | ImportStatus::Exists | ImportStatus::Deleted
            )
        })
        .count();
    let rejected = resp
        .results
        .iter()
        .filter(|result| matches!(result.status, ImportStatus::RepoNotAllowed))
        .collect::<Vec<_>>();
    let rejected_count = u32::try_from(rejected.len())
        .map_err(|_| Error::Other("repository rejection count exceeds u32".into()))?;
    stats.rejected_repo = stats.rejected_repo.saturating_add(rejected_count);
    if let Some(first) = rejected.first() {
        return Err(Error::Other(format!(
            "platform rejected {rejected_count} memory entries under repository policy (first: {})",
            first.id
        )));
    }
    let accepted = u32::try_from(accepted)
        .map_err(|_| Error::Other("accepted import count exceeds u32".into()))?;
    stats.pushed = stats.pushed.saturating_add(accepted);
    stats.last_pushed_seq = batch_high_seq;
    Ok(())
}

fn build_import_entry(
    store: &MemoryStore,
    conn: &Connection,
    log_row: &sync_log::SyncLogRow,
    wire: &Wire<'_>,
    stats: &mut PushStats,
) -> Result<Option<(ImportEntry, String)>> {
    let label = memory_repository::label(conn, &log_row.memory_id)?.unwrap_or_default();
    if wire.skip.is_skipped(&label) {
        stats.skipped_config += 1;
        return Ok(None);
    }
    let Some(repository) = wire.policy.memory_repository(&label).map(str::to_owned) else {
        stats.blocked_repo += 1;
        return Ok(None);
    };
    if log_row.op != SyncOp::Tombstone {
        let loaded = match store.load(&log_row.memory_id) {
            Ok(rec) => Some(rec),
            Err(Error::NotFound(_)) => None,
            Err(e) => return Err(e),
        };
        if matches!(log_row.op, SyncOp::Upsert | SyncOp::Restore)
            && let Some(rec) = loaded.as_ref()
            && redact::scan_with_override(conn, &log_row.memory_id, &rec.body)?.is_some()
        {
            stats.blocked_secrets += 1;
            return Ok(None);
        }
    }
    sync_binding::bind_first(conn, &log_row.memory_id, &wire.key.workspace_id)?;
    let record = match log_row.op {
        SyncOp::Tombstone => None,
        SyncOp::Upsert | SyncOp::Restore => enrich_record(store, conn, &log_row.memory_id)?,
    };
    Ok(Some((
        ImportEntry {
            op: log_row.op,
            id: log_row.memory_id.clone(),
            content_hash: log_row.content_hash.clone(),
            at: log_row.at.clone(),
            record,
        },
        repository,
    )))
}

fn now_iso() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("timestamp: {e}")))
}

#[cfg(test)]
#[path = "tests/push.rs"]
mod tests;
