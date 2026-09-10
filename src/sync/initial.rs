//! The first sync, run inside `comemory auth login`.
//!
//! Before organization scoping a fresh login left sync inert: the workspace
//! fell back to the personal one, which the platform rejects, so `[sync]
//! after_save` pushed into a void until the user found `comemory workspaces`
//! and `comemory link`. The key now names its workspace, so login can simply
//! do the first sync itself.
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

/// Entries moved in each direction by the login-time sync.
///
/// Bounded rather than exhaustive: a first login against a large organization
/// takes the most recent page and leaves the rest to the next `comemory sync`,
/// so approving a device never blocks on an unbounded transfer.
const INITIAL_LIMIT: usize = 500;

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

/// Pull, then push, against the organization `auth` is scoped to.
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

    let pulled = pull::run_pull(paths, cfg, &mut conn, auth, INITIAL_LIMIT)?;
    let pushed = push::run_push(paths, cfg, &mut conn, auth, None, INITIAL_LIMIT)?;
    Ok(InitialSyncStats {
        pulled: pulled.pulled,
        pushed: pushed.pushed,
        skipped_personal: pushed.skipped_personal,
        skipped_config: pushed.skipped_config,
    })
}

#[cfg(test)]
#[path = "tests/initial.rs"]
mod tests;
