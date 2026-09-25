//! `comemory sync daemon` — the required resident coordinator's lifecycle:
//! `ensure`, `status`, `restart`, `repair`, `stop`, `uninstall`, and the
//! foreground `run` the supervisor executes. `install`/`start` are hidden
//! deprecated aliases of `repair`/`ensure`.

use std::io::Write as _;

use clap::Subcommand;

use crate::cli::off_runtime::off_runtime;
use crate::cli::output::json;
use crate::config::Paths;
use crate::domains::sync::daemon::ensure::{self, Ensured, Intent};
use crate::domains::sync::daemon::{status_view, supervisor};
use crate::prelude::*;

/// `install`/`uninstall`/`start`/`stop`/`status`/`run` predate the required
/// daemon (#257); `ensure`/`restart`/`repair` are new.
#[derive(Subcommand, Debug)]
pub enum DaemonCmd {
    /// Verify and, if needed, repair and start the coordinator.
    Ensure,
    /// Report the coordinator's live state; starts nothing.
    Status,
    /// Gracefully stop the live coordinator, then ensure a fresh one.
    Restart,
    /// Rewrite this data directory's service definition and restart unless
    /// a verified current coordinator already runs.
    Repair,
    /// Foreground coordinator — the supervisor's entry point.
    Run,
    /// Gracefully stop the coordinator; the next ordinary command restarts it.
    Stop,
    /// Stop and remove this data directory's service definition; the next
    /// ordinary command reinstalls it.
    Uninstall,
    /// Deprecated alias of `repair`.
    #[command(hide = true)]
    Install,
    /// Deprecated alias of `ensure`.
    #[command(hide = true)]
    Start,
}

/// Dispatch a `sync daemon` subcommand. `run` is foreground and async; every
/// other verb does its blocking work off the runtime.
pub async fn run(paths: &Paths, cmd: DaemonCmd, json_flag: bool) -> Result<()> {
    if matches!(cmd, DaemonCmd::Run) {
        return crate::domains::sync::daemon::run_foreground(paths).await;
    }
    off_runtime(|| dispatch(paths, cmd, json_flag))
}

fn dispatch(paths: &Paths, cmd: DaemonCmd, json_flag: bool) -> Result<()> {
    match cmd {
        DaemonCmd::Ensure | DaemonCmd::Start => {
            emit_ensured(json_flag, ensure::ensure(paths, Intent::Ensure)?)
        }
        DaemonCmd::Restart => emit_ensured(json_flag, ensure::ensure(paths, Intent::Restart)?),
        DaemonCmd::Repair | DaemonCmd::Install => {
            emit_ensured(json_flag, ensure::ensure(paths, Intent::Repair)?)
        }
        DaemonCmd::Status => emit_status(json_flag, status_view::view(paths)?),
        DaemonCmd::Stop => stop(paths, json_flag),
        DaemonCmd::Uninstall => uninstall(paths, json_flag),
        DaemonCmd::Run => Err(Error::Other(
            "`sync daemon run` is dispatched before this match".into(),
        )),
    }
}

fn stop(paths: &Paths, json_flag: bool) -> Result<()> {
    let running = matches!(
        crate::domains::sync::daemon::client::probe(
            paths,
            crate::domains::sync::daemon::client::PROBE_BOUND
        ),
        crate::domains::sync::daemon::client::Probe::Healthy(_)
    );
    if running {
        let _ = crate::domains::sync::daemon::client::shutdown(
            paths,
            std::time::Duration::from_secs(15),
        );
    }
    report(
        json_flag,
        "stopped",
        "stopped (the next comemory command restarts a required daemon)",
    )
}

fn uninstall(paths: &Paths, json_flag: bool) -> Result<()> {
    stop(paths, false)?;
    let kind = supervisor::detect()?;
    let canonical = crate::domains::sync::daemon::identity::canonical_data_dir(paths)?;
    if kind != supervisor::Kind::Process && kind != supervisor::Kind::External {
        let unit = supervisor::plan(&canonical, kind)?;
        supervisor::remove(kind, &unit);
    }
    report(
        json_flag,
        "uninstalled",
        "uninstalled this data directory's service definition (the next comemory command reinstalls it)",
    )
}

/// `{"<key>": true}` under `--json`, else one TTY line.
fn report(json_flag: bool, key: &str, message: &str) -> Result<()> {
    if json_flag {
        return json::write(&serde_json::json!({ key: true }));
    }
    writeln!(std::io::stdout().lock(), "{message}")?;
    Ok(())
}

fn emit_ensured(json_flag: bool, ensured: Ensured) -> Result<()> {
    if json_flag {
        json::write(&serde_json::json!({
            "ready": ensured.ready,
            "action": ensured.action,
            "supervisor": ensured.supervisor.as_str(),
            "notes": ensured.notes,
            "daemon": ensured.daemon,
            "error": ensured.error,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        if ensured.ready {
            writeln!(
                out,
                "ready ({}, action={}, supervisor={})",
                ensured
                    .daemon
                    .as_ref()
                    .map_or_else(String::new, |d| format!("pid {}", d.pid)),
                ensured.action,
                ensured.supervisor.as_str()
            )?;
        } else {
            writeln!(
                std::io::stderr().lock(),
                "error: {}",
                ensured.error.as_deref().unwrap_or("not ready")
            )?;
        }
        for note in &ensured.notes {
            writeln!(out, "  note: {note}")?;
        }
    }
    if ensured.ready {
        Ok(())
    } else {
        Err(Error::Unavailable(
            ensured
                .error
                .unwrap_or_else(|| "sync daemon not ready".into()),
        ))
    }
}

fn emit_status(json_flag: bool, view: status_view::StatusView) -> Result<()> {
    if json_flag {
        return json::write(&view);
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", view.detail)?;
    if let Some(matches) = view.version_matches
        && !matches
    {
        writeln!(
            out,
            "  warning: coordinator version differs from this binary"
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/sync_daemon.rs"]
mod tests;
