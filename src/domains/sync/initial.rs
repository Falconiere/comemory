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
//!
//! The code index goes last, after memories: it is the larger payload and the
//! less urgent one, and a memory that cites a file is what makes the file's
//! node on the workspace worth having. Its failure never fails the login —
//! it is reported beside the memory counts and re-offered by the next sync.

use crate::config::{Config, Paths};
use crate::domains::sync::AuthFile;
use crate::domains::sync::auto::hold_pass_lock;
use crate::domains::sync::code::{self, CodePushStats};
use crate::domains::sync::drain::{
    self,
    session::{Legs, Mode},
};
use crate::prelude::*;
use crate::store::connection;

/// What the first sync moved, for the login report.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct InitialSyncStats {
    /// Entries applied from the organization.
    pub pulled: u32,
    /// Local entries accepted by the organization.
    pub pushed: u32,
    /// Local entries withheld by `[sync] skip_repos`.
    pub skipped_config: u32,
    /// Local entries withheld because no approved repository identity could be proved.
    pub blocked_repo: u32,
    /// The code-index push that follows the memory push — every indexed
    /// repo, so the console's graph fills in from this login on.
    pub code: CodePushStats,
    /// Why the code push could not run at all (store or config), when it
    /// could not; per-repo transport failures are inside `code.errors`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_error: Option<String>,
}

/// Drain the key `auth` names — pull first, then push, until a pass ends
/// without `more` — then, for a legacy key of a managed origin, push the code
/// index (a `replica-v1` pass carries code generations itself).
///
/// # Errors
/// Propagates store and configuration failures, and — as
/// [`Error::Unavailable`] — a drain that ended on the network (recorded on
/// the key too). Callers in the login path treat errors as non-fatal:
/// the credential is already on disk and useful, and a login that fails
/// because the network blipped leaves the user with nothing and no obvious
/// next step.
pub fn run_initial_sync(paths: &Paths, cfg: &Config, auth: &AuthFile) -> Result<InitialSyncStats> {
    paths.ensure_dirs()?;
    let _pass = hold_pass_lock(paths)?;
    let mut conn = connection::open(paths.db_path())?;
    crate::config::sync::apply_embed_model(&conn, &cfg.embed)?;
    let drained = drain::drain(paths, cfg, &mut conn, auth, (Mode::Manual, Legs::Both))?;
    if let Some(error) = drained.error {
        return Err(Error::Unavailable(format!("first sync: {error}")));
    }
    let mut stats = InitialSyncStats {
        pulled: drained.exchange.pulled,
        pushed: drained.exchange.pushed,
        ..InitialSyncStats::default()
    };
    let Some(legacy) = drained.legacy else {
        return Ok(stats);
    };
    let (pull, push) = (
        legacy.pull.unwrap_or_default(),
        legacy.push.unwrap_or_default(),
    );
    stats.pulled = pull.pulled;
    stats.pushed = push.pushed;
    stats.skipped_config = push.skipped_config;
    stats.blocked_repo = push.blocked_repo;
    if !drained.managed {
        return Ok(stats);
    }
    match code::run_code_push(cfg, &mut conn, auth) {
        Ok(code) => stats.code = code,
        Err(e) => {
            tracing::warn!(error = %e, "code index push at login failed");
            stats.code_error = Some(e.to_string());
        }
    }
    Ok(stats)
}

#[cfg(test)]
#[path = "tests/initial.rs"]
mod tests;
