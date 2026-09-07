//! Push local sync-log entries to the platform (allowlist + redaction filter).

use rusqlite::Connection;
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::api::sync::changes::enrich_record;
use crate::api::sync::{ImportEntry, ImportRequest, ImportStatus, SyncOp};
use crate::config::{Config, Paths};
use crate::memory::MemoryStore;
use crate::prelude::*;
use crate::store::{sync_binding, sync_log, sync_state};
use crate::sync::AuthFile;
use crate::sync::allowlist_cache::AllowlistCache;
use crate::sync::client;
use crate::sync::match_key::{MatchOutcome, classify_repo};
use crate::sync::redact;

const MAX_BATCH: usize = 500;

/// Counters surfaced by `comemory sync --status` and returned from push.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PushStats {
    /// Entries accepted by the platform in this run.
    pub pushed: u32,
    /// Skipped — empty/unbound repo label.
    pub skipped_personal: u32,
    /// Skipped — repo not on the org allowlist.
    pub skipped_not_in_org: u32,
    /// Skipped — ambiguous basename match.
    pub skipped_ambiguous: u32,
    /// Blocked — secret rule hit without override.
    pub blocked_secrets: u32,
    /// Highest local seq included in a successful batch.
    pub last_pushed_seq: i64,
}

/// Push local-origin log entries above `pushed_seq` for `workspace_id`.
pub fn run_push(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    auth: &AuthFile,
    workspace_id: &str,
    allow_secret_id: Option<&str>,
    limit: usize,
) -> Result<PushStats> {
    if let Some(id) = allow_secret_id {
        let at = OffsetDateTime::now_utc()
            .format(&Iso8601::DEFAULT)
            .map_err(|e| Error::Other(format!("timestamp: {e}")))?;
        sync_binding::allow_secret(conn, id, workspace_id, "cli_override", &at)?;
    }

    sync_state::ensure(conn, workspace_id, &auth.api_url)?;
    let row = sync_state::get(conn, workspace_id)?
        .ok_or_else(|| Error::Other("sync_state missing after ensure".into()))?;
    let allowlist = refresh_allowlist(paths, cfg, auth, workspace_id)?;
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
                build_import_entry(paths, conn, &log_row, &allowlist, workspace_id, &mut stats)?
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
        let resp = client::push_import(&auth.api_url, &secret, workspace_id, &req)?;
        stats.pushed += resp
            .results
            .iter()
            .filter(|r| {
                matches!(
                    r.status,
                    ImportStatus::Accepted | ImportStatus::Exists | ImportStatus::Deleted
                )
            })
            .count() as u32;
        stats.last_pushed_seq = batch_high_seq;
        sync_state::set_pushed(conn, workspace_id, batch_high_seq, &now_iso()?)?;
        if stats.pushed as usize >= cap {
            break;
        }
    }
    Ok(stats)
}

fn build_import_entry(
    paths: &Paths,
    conn: &Connection,
    log_row: &sync_log::SyncLogRow,
    allowlist: &[crate::sync::match_key::AllowlistRepo],
    workspace_id: &str,
    stats: &mut PushStats,
) -> Result<Option<ImportEntry>> {
    let store = MemoryStore::new(paths.clone());
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
        match classify_repo(&repo, allowlist) {
            MatchOutcome::SkippedPersonal => {
                stats.skipped_personal += 1;
                return Ok(None);
            }
            MatchOutcome::SkippedNotInOrg => {
                stats.skipped_not_in_org += 1;
                return Ok(None);
            }
            MatchOutcome::SkippedAmbiguous => {
                stats.skipped_ambiguous += 1;
                return Ok(None);
            }
            MatchOutcome::Allowed => {}
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
        SyncOp::Upsert | SyncOp::Restore => enrich_record(&store, conn, &log_row.memory_id)?,
    };
    Ok(Some(ImportEntry {
        op: log_row.op,
        id: log_row.memory_id.clone(),
        content_hash: log_row.content_hash.clone(),
        at: log_row.at.clone(),
        record,
    }))
}

fn refresh_allowlist(
    paths: &Paths,
    cfg: &Config,
    auth: &AuthFile,
    workspace_id: &str,
) -> Result<Vec<crate::sync::match_key::AllowlistRepo>> {
    let ttl = cfg.sync.allowlist_ttl_duration()?;
    if let Some(cache) = AllowlistCache::load(paths)?
        && cache.workspace_id == workspace_id
        && cache.is_fresh(ttl)
    {
        return Ok(cache.repos);
    }
    let prior = AllowlistCache::load(paths)?;
    let etag = prior.as_ref().and_then(|c| c.etag.clone());
    let secret = auth.effective_secret();
    let (repos, new_etag) =
        client::fetch_allowlist(&auth.api_url, &secret, workspace_id, etag.as_deref())?;
    // Empty repos + matching etag ⇒ keep prior cache; otherwise persist
    // (including an empty allowlist for personal / unbound workspaces).
    if repos.is_empty()
        && new_etag.is_some()
        && etag.as_deref() == new_etag.as_deref()
        && let Some(cache) = prior.filter(|c| c.workspace_id == workspace_id)
    {
        return Ok(cache.repos);
    }
    let cache = AllowlistCache {
        etag: new_etag.or(etag),
        fetched_at: OffsetDateTime::now_utc(),
        repos: repos.clone(),
        workspace_id: workspace_id.to_string(),
    };
    let _ = cache.save(paths);
    Ok(repos)
}

fn now_iso() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("timestamp: {e}")))
}

#[cfg(test)]
#[path = "tests/push.rs"]
mod tests;
