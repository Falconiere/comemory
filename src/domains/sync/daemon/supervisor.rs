//! Which OS supervisor keeps a data directory's coordinator running, and the
//! per-directory unit file that backend uses.
//!
//! One data directory, one identity: `io.comemory.sync.<id>` (launchd) or
//! `comemory-sync-<id>.service` (systemd --user), `<id>` from
//! [`super::identity::data_dir_id`]. `write`/`start`/`remove` share their
//! `launchctl`/`systemctl` plumbing with the deprecated single-daemon
//! surface in [`crate::domains::sync::daemon_unit`] rather than duplicating
//! it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::env;
use crate::domains::sync::daemon::identity::{UNIT_ID_LEN, data_dir_id};
use crate::domains::sync::daemon_templates::{render_launch_agent_plist, render_systemd_unit};
use crate::domains::sync::daemon_unit::{run_supervisor, users_uid};
use crate::prelude::*;

/// A legacy (un-id'd) unit this issue's per-directory scheme retires.
const LEGACY_LAUNCHD_LABEL: &str = "io.comemory.sync";
/// A legacy (un-id'd) systemd unit this issue's per-directory scheme retires.
const LEGACY_SYSTEMD_UNIT: &str = "comemory-sync.service";

/// Which backend keeps a coordinator running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// macOS user LaunchAgent.
    Launchd,
    /// Linux systemd `--user`.
    Systemd,
    /// A detached process this build supervises itself ([`super::spawn`]).
    Process,
    /// The operator's own supervisor; `ensure` only verifies.
    External,
    /// No backend for this OS at all.
    Unsupported,
}

impl Kind {
    /// The string readiness and `status` report.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Launchd => "launchd",
            Self::Systemd => "systemd",
            Self::Process => "process",
            Self::External => "external",
            Self::Unsupported => "unsupported",
        }
    }
}

/// `COMEMORY_DAEMON_SUPERVISOR`, else this OS's native backend, else — on
/// Linux only — [`Kind::Process`] when no user session bus is present.
///
/// # Errors
/// [`Error::Config`] for an unrecognized override value.
pub fn detect() -> Result<Kind> {
    if let Some(raw) = env::daemon_supervisor_override() {
        return match raw.to_ascii_lowercase().as_str() {
            "launchd" => Ok(Kind::Launchd),
            "systemd" => Ok(Kind::Systemd),
            "process" => Ok(Kind::Process),
            "external" => Ok(Kind::External),
            other => Err(Error::Config(format!(
                "COMEMORY_DAEMON_SUPERVISOR={other} is not launchd, systemd, process or external"
            ))),
        };
    }
    Ok(native())
}

#[cfg(target_os = "macos")]
fn native() -> Kind {
    Kind::Launchd
}

#[cfg(target_os = "linux")]
fn native() -> Kind {
    let has_bus = std::env::var_os("XDG_RUNTIME_DIR").is_some()
        && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some();
    if has_bus {
        Kind::Systemd
    } else {
        Kind::Process
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn native() -> Kind {
    Kind::Unsupported
}

/// This directory's unit identity and path for `kind` (meaningless for
/// [`Kind::Process`]/[`Kind::External`]/[`Kind::Unsupported`]).
pub struct Unit {
    /// launchd label or systemd unit name.
    pub name: String,
    /// Where the unit file lives.
    pub path: PathBuf,
}

/// This data directory's unit under `kind`.
///
/// # Errors
/// `$HOME` is unset.
pub fn plan(canonical: &Path, kind: Kind) -> Result<Unit> {
    let id = data_dir_id(canonical, UNIT_ID_LEN);
    let name = match kind {
        Kind::Launchd => format!("io.comemory.sync.{id}"),
        Kind::Systemd => format!("comemory-sync-{id}.service"),
        Kind::Process | Kind::External | Kind::Unsupported => {
            return Ok(Unit {
                name: id,
                path: PathBuf::new(),
            });
        }
    };
    named_unit(kind, name)
}

fn named_unit(kind: Kind, name: String) -> Result<Unit> {
    let root = home()?;
    let path = if kind == Kind::Launchd {
        root.join("Library/LaunchAgents")
            .join(format!("{name}.plist"))
    } else {
        root.join(".config/systemd/user").join(&name)
    };
    Ok(Unit { name, path })
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::Config("HOME is unset — cannot place the sync daemon unit".into()))
}

/// Write `unit`'s file for `exe`/`canonical`, then start (or kick a stale)
/// it; a no-op outside launchd/systemd — the caller spawns
/// [`Kind::Process`] itself (`super::spawn`). Every caller writes and starts
/// together, so there is one native round trip per backend, not two.
///
/// # Errors
/// The unit directory cannot be created or written, or a supervisor CLI
/// could not be invoked ([`Kind::External`] never gets here: `ensure`
/// verifies only).
pub fn activate(kind: Kind, unit: &Unit, exe: &Path, canonical: &Path) -> Result<()> {
    let content = match kind {
        Kind::Launchd => render_launch_agent_plist(&unit.name, exe, canonical),
        Kind::Systemd => render_systemd_unit(exe, canonical),
        Kind::Process | Kind::External | Kind::Unsupported => return Ok(()),
    };
    if let Some(parent) = unit.path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&unit.path, content)?;
    if kind == Kind::Launchd {
        activate_launchd(unit)
    } else {
        run_supervisor("systemctl", &["--user", "daemon-reload"]);
        let status = Command::new("systemctl")
            .args(["--user", "start", &unit.name])
            .status()
            .map_err(|e| Error::Other(format!("systemctl start: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::Other(format!(
                "systemctl --user start {} failed",
                unit.name
            )))
        }
    }
}

fn activate_launchd(unit: &Unit) -> Result<()> {
    let uid = users_uid()?;
    let domain = format!("gui/{uid}");
    if !bootstrap_launchd(unit, &domain)? {
        // An already-loaded label retains its old plist. Unload that exact
        // job before bootstrapping the rewritten definition.
        let status = Command::new("launchctl")
            .args(["bootout", &format!("{domain}/{}", unit.name)])
            .status()
            .map_err(|e| Error::Other(format!("launchctl bootout: {e}")))?;
        if !status.success() {
            return Err(Error::Other(format!(
                "launchctl bootout {} failed",
                unit.name
            )));
        }
        if !bootstrap_launchd(unit, &domain)? {
            return Err(Error::Other(format!(
                "launchctl bootstrap {} failed",
                unit.name
            )));
        }
    }
    Ok(())
}

fn bootstrap_launchd(unit: &Unit, domain: &str) -> Result<bool> {
    Command::new("launchctl")
        .args(["bootstrap", domain, &unit.path.display().to_string()])
        .status()
        .map(|status| status.success())
        .map_err(|e| Error::Other(format!("launchctl bootstrap: {e}")))
}

/// Stop the unit without removing it; best-effort.
pub fn stop(kind: Kind, unit: &Unit) {
    match kind {
        Kind::Launchd => match users_uid() {
            Ok(uid) => run_supervisor(
                "launchctl",
                &["bootout", &format!("gui/{uid}/{}", unit.name)],
            ),
            Err(e) => tracing::warn!(error = %e, "sync daemon stop skipped"),
        },
        Kind::Systemd => run_supervisor("systemctl", &["--user", "stop", &unit.name]),
        Kind::Process | Kind::External | Kind::Unsupported => {}
    }
}

/// Remove the unit file (stopped first).
pub fn remove(kind: Kind, unit: &Unit) {
    stop(kind, unit);
    match kind {
        Kind::Launchd | Kind::Systemd => {
            if unit.path.exists()
                && let Err(e) = fs::remove_file(&unit.path)
            {
                tracing::warn!(error = %e, path = %unit.path.display(), "sync daemon unit removal failed");
            }
            if kind == Kind::Systemd {
                run_supervisor("systemctl", &["--user", "daemon-reload"]);
            }
        }
        Kind::Process | Kind::External | Kind::Unsupported => {}
    }
}

/// Boot out and remove the pre-#257 single-daemon unit, if any, so a
/// repaired directory never runs two coordinators under two labels.
pub fn remove_legacy(kind: Kind) {
    if let Some(unit) = legacy_unit(kind) {
        remove(kind, &unit);
    }
}

/// The pre-#257 un-id'd unit's name and path, when `kind` has one.
fn legacy_unit(kind: Kind) -> Option<Unit> {
    let name = match kind {
        Kind::Launchd => LEGACY_LAUNCHD_LABEL,
        Kind::Systemd => LEGACY_SYSTEMD_UNIT,
        Kind::Process | Kind::External | Kind::Unsupported => return None,
    };
    named_unit(kind, name.into()).ok()
}

#[cfg(test)]
#[path = "tests/supervisor.rs"]
mod tests;
