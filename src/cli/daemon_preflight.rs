//! Before an ordinary command runs, verify (and if needed repair) the
//! required resident coordinator for its data directory (A-3, A-7).
//!
//! An exhaustive `match` over [`Cmd`] classifies every subcommand so a new
//! one added later must be classified explicitly rather than silently
//! defaulting to one bucket or the other. `--help`, `--version` and a clap
//! parse error never reach here — they exit inside [`Cli::parse`] before
//! [`run`] is called.
//!
//! [`Cli::parse`]: clap::Parser::parse

use std::path::Path;
use std::time::Duration;

use crate::cli::auth::AuthCmd;
use crate::cli::sync::SyncAction;
use crate::cli::sync::SyncCmd;
use crate::cli::{Cmd, load_config};
use crate::config::paths::{Paths, resolve_data_dir};
use crate::config::sync::daemon_disabled;
use crate::domains::sync::daemon::ensure::{self, Intent};
use crate::prelude::*;

/// How preflight treats one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Classification {
    /// No preflight: `serve`/`daemon run` are already supervised owners,
    /// lifecycle/introspection commands must not recurse into `ensure`.
    Exempt,
    /// Verify and repair, but never fail the command on an unreachable
    /// coordinator — `auth logout` must still remove a credential (D11).
    BestEffort,
    /// Verify and repair; an unreachable coordinator fails the command with
    /// an actionable local-service error (A-3).
    Required,
}

/// Classify `cmd`. Exhaustive so a future [`Cmd`] variant is a compile error
/// here, not a silent default.
fn classify(cmd: &Cmd) -> Classification {
    match cmd {
        Cmd::Serve(_) | Cmd::Completions(_) | Cmd::Upgrade(_) | Cmd::Doctor(_) => {
            Classification::Exempt
        }
        Cmd::Auth(a) => classify_auth(&a.cmd),
        Cmd::Sync(a) => classify_sync(a),
        Cmd::Architecture(_)
        | Cmd::Save(_)
        | Cmd::Search(_)
        | Cmd::SearchCode(_)
        | Cmd::List(_)
        | Cmd::Delete(_)
        | Cmd::Distill(_)
        | Cmd::Feedback(_)
        | Cmd::Eval(_)
        | Cmd::Benchmark(_)
        | Cmd::Judge(_)
        | Cmd::ExportDataset(_)
        | Cmd::Mine(_)
        | Cmd::Tune(_)
        | Cmd::Bandit(_)
        | Cmd::IndexCode(_)
        | Cmd::IngestCode(_)
        | Cmd::Index(_)
        | Cmd::Sources(_)
        | Cmd::Stats(_)
        | Cmd::Repos(_)
        | Cmd::Show(_)
        | Cmd::Find(_)
        | Cmd::Hooks(_)
        | Cmd::Unindex(_)
        | Cmd::Ast(_)
        | Cmd::Graph(_)
        | Cmd::Edges(_)
        | Cmd::Mcp(_)
        | Cmd::Setup(_)
        | Cmd::Context(_)
        | Cmd::Prune(_)
        | Cmd::Consolidate(_)
        | Cmd::Rebuild(_)
        | Cmd::RecallStatus(_)
        | Cmd::Gc
        | Cmd::InstallHooks(_)
        | Cmd::Install(_)
        | Cmd::Watch(_)
        | Cmd::Capture(_) => Classification::Required,
    }
}

fn classify_auth(cmd: &AuthCmd) -> Classification {
    match cmd {
        AuthCmd::Status(_) => Classification::Exempt,
        AuthCmd::Logout => Classification::BestEffort,
        AuthCmd::Login(_) => Classification::Required,
    }
}

fn classify_sync(args: &crate::cli::sync::Args) -> Classification {
    match &args.cmd {
        Some(SyncCmd::Daemon(_)) => Classification::Exempt,
        None if matches!(args.action, SyncAction::Status) => Classification::Exempt,
        _ => Classification::Required,
    }
}

/// How long the probe alone may take before preflight gives up and repairs.
const PROBE_BOUND: Duration = Duration::from_secs(2);

/// Verify/repair the coordinator for `cmd`'s data directory, per
/// [`classify`]. A no-op when `COMEMORY_SYNC_DAEMON=0` (the test/CI harness
/// switch) or the command is exempt.
///
/// # Errors
/// [`Error::Unavailable`] naming the fix when a required coordinator cannot
/// be reached within its bound. `BestEffort` commands never fail here.
pub fn run(data_dir: Option<&Path>, cmd: &Cmd) -> Result<()> {
    if daemon_disabled() {
        return Ok(());
    }
    let classification = classify(cmd);
    if classification == Classification::Exempt {
        return Ok(());
    }
    let paths = Paths::new(resolve_data_dir(data_dir.map(Path::to_path_buf)));
    if classification == Classification::Required {
        // Fail fast on a broken config.toml, same as the command itself
        // would — but never for BestEffort (D11): a broken config must not
        // block `auth logout` from still removing the credential.
        load_config(&paths)?;
    }
    let quick = quick_probe(&paths);
    if quick {
        return Ok(());
    }
    let ensured = match ensure::ensure(&paths, Intent::Preflight) {
        Ok(ensured) => ensured,
        Err(error) if classification == Classification::BestEffort => {
            tracing::warn!(%error, "sync daemon preflight failed; continuing logout");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    if ensured.ready || classification == Classification::BestEffort {
        return Ok(());
    }
    Err(Error::Unavailable(
        ensured
            .error
            .unwrap_or_else(|| "sync daemon not ready".into()),
    ))
}

/// A bare probe, bounded to [`PROBE_BOUND`] — the common case (an already
/// verified coordinator) never pays `ensure`'s lock/repair machinery.
fn quick_probe(paths: &Paths) -> bool {
    matches!(
        crate::domains::sync::daemon::client::probe(paths, PROBE_BOUND),
        crate::domains::sync::daemon::client::Probe::Healthy(_)
    )
}

#[cfg(test)]
#[path = "tests/daemon_preflight.rs"]
mod tests;
