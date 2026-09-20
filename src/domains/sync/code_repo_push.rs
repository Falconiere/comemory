//! One canonical repository's manifest diff, batch upload, and local cursor.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::domains::sync::code::{CodePushStats, project_file};
use crate::domains::sync::exchange::CoChangeWire;
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::domains::sync::{AuthFile, client_code, code_plan};
use crate::prelude::*;
use crate::store::code_sync::{self, CodeSyncCursor};
use crate::store::{Connection, indexed_files, repo_marker};

struct LocalState {
    head: Option<String>,
    mined: Option<String>,
    files: Vec<(String, String)>,
    digest: String,
}

/// Diff and upload one local index under its canonical platform identity.
pub fn push_repo(
    conn: &Connection,
    auth: &AuthFile,
    policy: &RepositoryPolicy,
    local_repo: &str,
    canonical_repo: &str,
    force: bool,
    stats: &mut CodePushStats,
) -> Result<()> {
    let local = local_state(conn, local_repo)?;
    if !force && cursor_matches(conn, local_repo, &local)? {
        stats.unchanged += 1;
        return Ok(());
    }
    let secret = auth.effective_secret();
    let manifest = client_code::fetch_code_manifest(
        &auth.api_url,
        &secret,
        canonical_repo,
        policy.revision(),
    )?;
    let plan = code_plan::plan(
        &local.files,
        &manifest.files,
        local.head.as_deref(),
        manifest.head.as_deref(),
        local.mined.as_deref(),
        manifest.mined_commit.as_deref(),
    );
    if plan.is_empty() {
        record_cursor(conn, local_repo, &local)?;
        stats.unchanged += 1;
        return Ok(());
    }
    let known: BTreeSet<&str> = local.files.iter().map(|(path, _)| path.as_str()).collect();
    let files = plan
        .changed
        .iter()
        .map(|path| project_file(conn, local_repo, path, &known))
        .collect::<Result<Vec<_>>>()?;
    let cochange = if plan.send_cochange {
        Some(
            code_sync::co_changed_pairs(conn, local_repo)?
                .into_iter()
                .map(|(from, to, weight)| CoChangeWire { from, to, weight })
                .collect(),
        )
    } else {
        None
    };
    let requests = code_plan::batches(
        canonical_repo,
        local.head.as_deref(),
        local.mined.as_deref(),
        files,
        plan.removed,
        cochange,
    )?;
    for req in &requests {
        let resp = client_code::push_code_import(&auth.api_url, &secret, req, policy.revision())?;
        if let Some(first) = resp.rejected.first() {
            return Err(Error::Other(format!(
                "workspace rejected the code import ({} entries; first: {} {})",
                resp.rejected.len(),
                first.path,
                first.reason
            )));
        }
        stats.files_pushed = stats.files_pushed.saturating_add(count(resp.applied));
        stats.files_removed = stats.files_removed.saturating_add(count(resp.removed));
        stats.batches += 1;
    }
    stats.repos += 1;
    record_cursor(conn, local_repo, &local)
}

fn local_state(conn: &Connection, repo: &str) -> Result<LocalState> {
    let files = indexed_files::list_for_repo(conn, repo)?;
    let mut hasher = Sha256::new();
    for (path, oid) in &files {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(oid.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        });
    Ok(LocalState {
        head: repo_marker::last_head(conn, repo)?,
        mined: repo_marker::last_mined_commit(conn, repo)?,
        files,
        digest,
    })
}

fn cursor_matches(conn: &Connection, repo: &str, local: &LocalState) -> Result<bool> {
    Ok(code_sync::cursor(conn, repo)?.is_some_and(|cursor| {
        cursor.pushed_head == local.head
            && cursor.pushed_mined_commit == local.mined
            && cursor.pushed_digest == local.digest
    }))
}

fn record_cursor(conn: &Connection, repo: &str, local: &LocalState) -> Result<()> {
    let pushed_at = OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("timestamp: {e}")))?;
    code_sync::set_cursor(
        conn,
        repo,
        &CodeSyncCursor {
            pushed_head: local.head.clone(),
            pushed_mined_commit: local.mined.clone(),
            pushed_digest: local.digest.clone(),
            pushed_at,
        },
    )
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
