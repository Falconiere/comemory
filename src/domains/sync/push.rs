//! Push local sync-log entries to the platform.
//!
//! One client-side filter remains: a label matching `[sync] skip_repos` is
//! withheld by the operator's own choice. Everything else is offered to the
//! organization, which decides.
//!
//! The rule that an unlabelled memory never left the machine is gone
//! (`2026-09-14-sync-everything-realtime-design.md`). `repo` comes from
//! `git2::Repository::discover` at save time, so that rule silently made sync
//! eligibility depend on which directory `comemory save` ran in: the same note
//! synced from inside a worktree and was stranded forever from anywhere else.
//! A label is metadata about where work happened, not a permission.

use std::time::Duration;

use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::config::{Config, Paths};
use crate::domains::memories::MemoryStore;
use crate::domains::sync::AuthFile;
use crate::domains::sync::client;
use crate::domains::sync::exchange::changes::enrich_record;
use crate::domains::sync::exchange::{ImportEntry, ImportRequest, ImportStatus, SyncOp};
use crate::domains::sync::redact;
use crate::domains::sync::skip_repos::SkipMatcher;
use crate::prelude::*;
use crate::store::{Connection, sync_binding, sync_log, sync_state};

const MAX_BATCH: usize = 500;

/// Counters surfaced by `comemory sync --status` and returned from push.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PushStats {
    /// Entries accepted by the platform in this run.
    pub pushed: u32,
    /// Skipped — label matched `[sync] skip_repos`.
    pub skipped_config: u32,
    /// Blocked — secret rule hit without override.
    pub blocked_secrets: u32,
    /// Rejected by the platform allowlist gate (`repo_not_allowed`).
    ///
    /// These do **not** advance `pushed_seq` when they are the only outcomes
    /// in a batch — otherwise retries would never re-offer the same seqs.
    pub rejected_repo: u32,
    /// Highest local seq included in a successful batch.
    pub last_pushed_seq: i64,
}

/// Push local-origin log entries above `pushed_seq` to the organization the
/// key in `auth` is scoped to.
///
/// # Errors
/// Propagates store, markdown and platform failures. An invalid
/// `[sync] skip_repos` glob is [`Error::Config`].
pub fn run_push(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    auth: &AuthFile,
    allow_secret_id: Option<&str>,
    limit: usize,
) -> Result<PushStats> {
    run_push_with_timeout(
        paths,
        cfg,
        conn,
        auth,
        allow_secret_id,
        limit,
        client::HTTP_TIMEOUT,
    )
}

/// Same as [`run_push`] under an explicit per-request timeout.
///
/// The inline push-on-save hook runs on a far smaller budget than a manual
/// sync: a save must return even when the network accepts the connection and
/// then says nothing.
///
/// # Errors
/// Propagates store, markdown and platform failures. An invalid
/// `[sync] skip_repos` glob is [`Error::Config`].
pub fn run_push_with_timeout(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    auth: &AuthFile,
    allow_secret_id: Option<&str>,
    limit: usize,
    timeout: Duration,
) -> Result<PushStats> {
    let workspace_id = auth.workspace_id.as_str();
    if let Some(id) = allow_secret_id {
        let at = OffsetDateTime::now_utc()
            .format(&Iso8601::DEFAULT)
            .map_err(|e| Error::Other(format!("timestamp: {e}")))?;
        sync_binding::allow_secret(conn, id, workspace_id, "cli_override", &at)?;
    }

    sync_state::ensure(conn, workspace_id, &auth.api_url)?;
    // Older corpora can have live memories never appended to `sync_log`
    // (push only drains the log). Best-effort backfill before the outbox walk.
    sync_log::backfill_missing_local(conn)?;
    let row = sync_state::get(conn, workspace_id)?
        .ok_or_else(|| Error::Other("sync_state missing after ensure".into()))?;
    let skip = cfg.sync.skip_matcher()?;
    let store = MemoryStore::new(paths.clone());
    let mut stats = PushStats::default();
    let mut since = row.pushed_seq;
    let cap = limit.min(MAX_BATCH * 4);

    loop {
        let rows = sync_log::local_entries_since(conn, since, MAX_BATCH)?;
        if rows.is_empty() {
            break;
        }
        let mut batch: Vec<ImportEntry> = Vec::new();
        let mut batch_high_seq = since;
        for log_row in rows {
            batch_high_seq = log_row.seq;
            if let Some(entry) =
                build_import_entry(&store, conn, &log_row, &skip, workspace_id, &mut stats)?
            {
                batch.push(entry);
            }
        }
        since = batch_high_seq;
        if batch.is_empty() {
            if stats.pushed == 0 && since >= row.pushed_seq {
                sync_state::set_pushed(conn, workspace_id, since, &now_iso()?)?;
            }
            continue;
        }
        let req = ImportRequest {
            cursor: row.pulled_seq,
            entries: batch,
        };
        let secret = auth.effective_secret();
        let resp = client::push_import_with(&auth.api_url, &secret, &req, timeout)?;
        let accepted = resp
            .results
            .iter()
            .filter(|r| {
                matches!(
                    r.status,
                    ImportStatus::Accepted | ImportStatus::Exists | ImportStatus::Deleted
                )
            })
            .count();
        let rejected_repo = resp
            .results
            .iter()
            .filter(|r| matches!(r.status, ImportStatus::RepoNotAllowed))
            .count();
        stats.rejected_repo =
            stats
                .rejected_repo
                .saturating_add(u32::try_from(rejected_repo).map_err(|_| {
                    Error::Other(format!(
                        "rejected_repo count not representable as u32: {rejected_repo}"
                    ))
                })?);
        stats.pushed = stats
            .pushed
            .saturating_add(u32::try_from(accepted).map_err(|_| {
                Error::Other(format!(
                    "accepted count not representable as u32: {accepted}"
                ))
            })?);
        // Withhold `pushed_seq` whenever any result is `repo_not_allowed`
        // (including a hypothetical mixed batch). Other terminal statuses
        // (stale, collision, …) with zero gate rejects still advance so we
        // do not retry forever. Retries of already-stored ids come back
        // `exists`.
        if rejected_repo == 0 {
            stats.last_pushed_seq = batch_high_seq;
            sync_state::set_pushed(conn, workspace_id, batch_high_seq, &now_iso()?)?;
        }
        if stats.pushed as usize >= cap {
            break;
        }
    }
    Ok(stats)
}

fn build_import_entry(
    store: &MemoryStore,
    conn: &Connection,
    log_row: &sync_log::SyncLogRow,
    skip: &SkipMatcher,
    workspace_id: &str,
    stats: &mut PushStats,
) -> Result<Option<ImportEntry>> {
    if log_row.op != SyncOp::Tombstone {
        let loaded = match store.load(&log_row.memory_id) {
            Ok(rec) => Some(rec),
            Err(Error::NotFound(_)) => None,
            Err(e) => return Err(e),
        };
        let repo = loaded
            .as_ref()
            .map(|rec| rec.frontmatter.repo.clone())
            .unwrap_or_default();
        // Every memory is offered to the organization unless the operator
        // withheld its label. An empty label matches nothing, so a memory
        // saved outside a worktree is offered like any other.
        if skip.is_skipped(&repo) {
            stats.skipped_config += 1;
            return Ok(None);
        }
        if matches!(log_row.op, SyncOp::Upsert | SyncOp::Restore)
            && let Some(rec) = loaded.as_ref()
            && redact::scan_with_override(conn, &log_row.memory_id, &rec.body)?.is_some()
        {
            stats.blocked_secrets += 1;
            return Ok(None);
        }
    }
    sync_binding::bind_first(conn, &log_row.memory_id, workspace_id)?;
    let record = match log_row.op {
        SyncOp::Tombstone => None,
        SyncOp::Upsert | SyncOp::Restore => enrich_record(store, conn, &log_row.memory_id)?,
    };
    Ok(Some(ImportEntry {
        op: log_row.op,
        id: log_row.memory_id.clone(),
        content_hash: log_row.content_hash.clone(),
        at: log_row.at.clone(),
        record,
    }))
}

fn now_iso() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("timestamp: {e}")))
}

#[cfg(test)]
#[path = "tests/push.rs"]
mod tests;
