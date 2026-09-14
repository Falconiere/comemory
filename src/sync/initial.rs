//! The first sync, run inside `comemory auth login`.
//!
//! Before organization scoping a fresh login left sync inert: the workspace
//! fell back to the personal one, which the platform rejects, so in-process
//! after-save pushes went into a void until the user found `comemory
//! workspaces` and `comemory link`. The key now names its workspace, so login
//! can simply do the first sync itself — exhaustively, then hand continuous
//! sync to the user-level daemon (`sync daemon`).
//!
//! Pull precedes push so a new machine adopts what the organization already
//! holds before offering its own copies — `sync_binding` pins a memory to the
//! workspace that first accepted it, and pushing first would claim memories
//! the organization already has.

use crate::config::{Config, Paths};
use crate::prelude::*;
use crate::store::{connection, sync_state};
use crate::sync::AuthFile;
use crate::sync::{pull, push};

/// Page size for each `run_pull` / `run_push` call inside the exhaustive loop.
///
/// Matches `comemory sync --action run`. The outer loop repeats until a page
/// moves nothing, so a large org corpus is not truncated at login.
const PAGE_LIMIT: usize = 2000;

/// What the first sync moved, for the login report.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct InitialSyncStats {
    /// Entries applied from the organization.
    pub pulled: u32,
    /// Local entries accepted by the organization.
    pub pushed: u32,
    /// Local entries withheld because they carry no repo label.
    pub skipped_personal: u32,
    /// Local entries withheld by `[sync] skip_repos`.
    pub skipped_config: u32,
}

/// Pull, then push, against the organization `auth` is scoped to — looping
/// until each direction returns an empty page.
///
/// # Errors
/// Propagates store and platform failures. Callers in the login path treat
/// them as non-fatal: the credential is already on disk and useful, and a
/// login that fails because the network blipped leaves the user with nothing
/// and no obvious next step.
pub fn run_initial_sync(paths: &Paths, cfg: &Config, auth: &AuthFile) -> Result<InitialSyncStats> {
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    crate::config::sync::apply_embed_model(&conn, &cfg.embed)?;
    sync_state::ensure(&conn, &auth.workspace_id, &auth.api_url)?;

    let mut stats = InitialSyncStats::default();
    loop {
        let page = pull::run_pull(paths, cfg, &mut conn, auth, PAGE_LIMIT)?;
        stats.pulled = stats.pulled.saturating_add(page.pulled);
        if page.pulled == 0 {
            break;
        }
    }
    loop {
        let page = push::run_push(paths, cfg, &mut conn, auth, None, PAGE_LIMIT)?;
        stats.pushed = stats.pushed.saturating_add(page.pushed);
        stats.skipped_personal = stats.skipped_personal.saturating_add(page.skipped_personal);
        stats.skipped_config = stats.skipped_config.saturating_add(page.skipped_config);
        if page.pushed == 0 {
            // Skips are drained inside `run_push` without counting toward the
            // page cap; a zero-push page means the local log is exhausted.
            break;
        }
    }
    Ok(stats)
}

#[cfg(test)]
#[path = "tests/initial.rs"]
mod tests;
