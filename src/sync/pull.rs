//! Pull platform sync entries and apply them locally via `api::sync::import`.

use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::api::sync::{ImportEntry, ImportRequest};
use crate::api::{self, Ctx};
use crate::config::{Config, Paths};
use crate::prelude::*;
use crate::store::{Connection, sync_state};
use crate::sync::AuthFile;
use crate::sync::client;

const MAX_BATCH: usize = 500;

/// Outcome counters for a pull run.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PullStats {
    /// Entries applied from the remote log.
    pub pulled: u32,
    /// Highest server seq applied locally.
    pub last_pulled_seq: i64,
}

/// Pull remote changes above `pulled_seq` and import them locally.
pub fn run_pull(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    auth: &AuthFile,
    limit: usize,
) -> Result<PullStats> {
    let workspace_id = auth.workspace_id.as_str();
    sync_state::ensure(conn, workspace_id, &auth.api_url)?;
    let row = sync_state::get(conn, workspace_id)?
        .ok_or_else(|| Error::Other("sync_state missing after ensure".into()))?;
    let mut stats = PullStats::default();
    let mut since = row.pulled_seq;
    let secret = auth.effective_secret();
    let cap = limit.min(MAX_BATCH * 4);

    loop {
        let remote = client::pull_changes(&auth.api_url, &secret, since, MAX_BATCH)?;
        if remote.entries.is_empty() {
            stats.last_pulled_seq = remote.head_seq.max(since);
            break;
        }
        let entries: Vec<ImportEntry> = remote
            .entries
            .iter()
            .map(|e| ImportEntry {
                op: e.op,
                id: e.id.clone(),
                content_hash: e.content_hash.clone(),
                at: e.at.clone(),
                record: e.record.clone(),
            })
            .collect();
        let high = if let Some(entry) = remote.entries.last() {
            entry.seq
        } else {
            since
        };
        let req = ImportRequest {
            cursor: since,
            entries,
        };
        let mut ctx = Ctx::borrowed(paths, cfg, conn);
        let _resp = api::sync::import::run(&mut ctx, req, None)?;
        stats.pulled += remote.entries.len() as u32;
        since = high;
        stats.last_pulled_seq = since;
        sync_state::set_pulled(conn, workspace_id, since, &now_iso()?)?;
        if stats.pulled as usize >= cap {
            break;
        }
        if remote.entries.len() < MAX_BATCH {
            break;
        }
    }
    Ok(stats)
}

fn now_iso() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("timestamp: {e}")))
}

#[cfg(test)]
#[path = "tests/pull.rs"]
mod tests;
