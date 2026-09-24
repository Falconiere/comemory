//! `comemory sync --action auto` — the unattended pass git hooks and the
//! agent `SessionStart` hook fire from any cwd: index the triggering checkout,
//! re-index every other stale hooked repo, then (logged in) pull, push, and
//! push moved code. Passes serialize on [`PASS_LOCK`]; a trigger that finds a
//! pass already queued ([`QUEUE_LOCK`]) exits, unless a later sweep would not
//! cover its checkout.

use std::path::Path;

use crate::config::sync::apply_embed_model;
use crate::config::{Config, Paths};
use crate::domains::code::hooked_refresh::{self, Checkout, RefreshStats};
use crate::domains::sync::AuthFile;
use crate::domains::sync::manual::{RUN_LIMIT, RunStats};
use crate::domains::sync::{code, pull, push};
use crate::prelude::*;
use crate::store::{Connection, connection};
use crate::utilities::file_lock::FileLock;

/// Lock file under the data directory held for the whole of one sync pass —
/// an `--action auto` pass, a manual `run`/`push`, or a daemon cycle — so two
/// passes never index or exchange at once.
pub const PASS_LOCK: &str = "sync.lock";

/// The one-slot queue in front of [`PASS_LOCK`]: an `--action auto` trigger
/// that finds it held knows a pass is already waiting and exits.
pub const QUEUE_LOCK: &str = "sync-auto.queue";

/// Block until this process holds [`PASS_LOCK`]; released on drop.
///
/// # Errors
/// The lock file cannot be opened or locked.
pub fn hold_pass_lock(paths: &Paths) -> Result<FileLock> {
    FileLock::acquire(&paths.data_dir().join(PASS_LOCK), "sync")
}

/// What one pass did.
#[derive(Debug, Default)]
pub struct AutoStats {
    /// Whether a usable credential was found; `false` means the network legs
    /// did not run.
    pub logged_in: bool,
    /// The refresh (always `Some`) and, when logged in, each network leg that
    /// succeeded.
    pub run: RunStats,
    /// The first network-leg failure, if any. The pass still ran every leg:
    /// an offline pull does not stop the local refresh from having happened.
    pub error: Option<String>,
}

/// How an `--action auto` invocation ended.
#[derive(Debug)]
pub enum AutoOutcome {
    /// A pass was already queued behind the running one and will do this
    /// trigger's work; nothing ran here.
    Coalesced,
    /// This invocation ran a pass.
    Ran(AutoStats),
}

/// Run one auto pass for the checkout at `checkout_path` (the hook's
/// `--path`), or for no particular checkout (the agent hook).
///
/// # Errors
/// Lock-file, store and configuration failures. Index and network failures
/// are reported in the returned stats instead.
pub fn run_auto(paths: &Paths, cfg: &Config, checkout_path: Option<&Path>) -> Result<AutoOutcome> {
    paths.ensure_dirs()?;
    let checkout = checkout_path.and_then(hooked_refresh::resolve_checkout);
    let must_run = match &checkout {
        Some(c) => !hooked_refresh::swept_by_a_later_pass(&connection::open(paths.db_path())?, c)?,
        None => false,
    };
    let queued = if must_run {
        None
    } else {
        match FileLock::try_acquire(&paths.data_dir().join(QUEUE_LOCK), "sync-queue")? {
            Some(slot) => Some(slot),
            None => return Ok(AutoOutcome::Coalesced),
        }
    };
    let _pass = hold_pass_lock(paths)?;
    drop(queued);

    let mut refresh = RefreshStats::default();
    if let (Some(path), None) = (checkout_path, &checkout) {
        refresh.record_failure(&path.display().to_string(), "not inside a git work tree");
    }
    let mut conn = connection::open(paths.db_path())?;
    run_pass(paths, cfg, &mut conn, checkout.as_ref(), refresh).map(AutoOutcome::Ran)
}

/// The pass itself, for a caller that already holds [`PASS_LOCK`] (this
/// module's [`run_auto`], the daemon cycle).
/// `refresh` carries anything already counted before the pass began.
///
/// # Errors
/// Store and configuration failures.
pub fn run_pass(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    checkout: Option<&Checkout>,
    mut refresh: RefreshStats,
) -> Result<AutoStats> {
    if let Some(c) = checkout {
        hooked_refresh::index_checkout(paths, cfg, conn, c, &mut refresh)?;
    }
    let skip = checkout.map(|c| c.label.as_str());
    hooked_refresh::refresh_stale(paths, cfg, conn, skip, &mut refresh)?;
    let mut stats = AutoStats {
        run: RunStats {
            refresh: Some(refresh),
            ..RunStats::default()
        },
        ..AutoStats::default()
    };
    let Some(auth) = AuthFile::load_usable(paths)? else {
        return Ok(stats);
    };
    stats.logged_in = true;
    apply_embed_model(conn, &cfg.embed)?;
    stats.run.pull = note(
        &mut stats.error,
        "pull",
        pull::run_pull(paths, cfg, conn, &auth, RUN_LIMIT),
    );
    stats.run.push = note(
        &mut stats.error,
        "push",
        push::run_push(paths, cfg, conn, &auth, None, RUN_LIMIT),
    );
    stats.run.code = note(
        &mut stats.error,
        "code push",
        code::run_code_push_if_moved(cfg, conn, &auth),
    );
    Ok(stats)
}

/// Keep a leg's stats, or log its failure and remember the first one.
fn note<T>(first: &mut Option<String>, leg: &str, outcome: Result<T>) -> Option<T> {
    match outcome {
        Ok(stats) => Some(stats),
        Err(e) => {
            tracing::warn!(leg, error = %e, "sync pass leg failed");
            first.get_or_insert_with(|| format!("{leg}: {e}"));
            None
        }
    }
}

#[cfg(test)]
#[path = "tests/auto.rs"]
mod tests;
