//! Pull platform sync entries and apply them locally via `domains::sync::exchange::import`.

use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::config::{Config, Paths};
use crate::domains::sync::AuthFile;
use crate::domains::sync::client;
use crate::domains::sync::exchange::{ImportEntry, ImportRequest};
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::prelude::*;
use crate::store::{Connection, sync_state};
use crate::utilities::context::Ctx;

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
    let policy = RepositoryPolicy::load(conn, auth)?;
    sync_state::ensure(conn, workspace_id, &auth.api_url)?;
    let row = sync_state::get(conn, workspace_id)?
        .ok_or_else(|| Error::Other("sync_state missing after ensure".into()))?;
    let mut stats = PullStats::default();
    let mut since = row.pulled_seq;
    let secret = auth.effective_secret();
    let cap = limit.min(MAX_BATCH * 4);

    loop {
        let remote =
            client::pull_changes(&auth.api_url, &secret, since, MAX_BATCH, policy.revision())?;
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
        if !entries.is_empty() {
            let req = ImportRequest {
                cursor: since,
                entries,
            };
            let mut ctx = Ctx::borrowed(paths, cfg, conn);
            let _resp = crate::domains::sync::exchange::import::run(&mut ctx, req, None)?;
            stats.pulled = stats.pulled.saturating_add(remote.entries.len() as u32);
        }
        let Some(next) = remote.next_seq else {
            stats.last_pulled_seq = since;
            break;
        };
        if next <= since {
            return Err(Error::Other(format!(
                "pull changes returned non-advancing next_seq {next} after {since}"
            )));
        }
        since = next;
        stats.last_pulled_seq = next;
        sync_state::set_pulled(conn, workspace_id, next, &now_iso()?)?;
        if stats.pulled as usize >= cap {
            break;
        }
        if next >= remote.head_seq {
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
