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
    match kind {
        Kind::Launchd => {
            let name = format!("io.comemory.sync.{id}");
            let path = home()?
                .join("Library/LaunchAgents")
                .join(format!("{name}.plist"));
            Ok(Unit { name, path })
        }
        Kind::Systemd => {
            let name = format!("comemory-sync-{id}.service");
            let path = home()?.join(".config/systemd/user").join(&name);
            Ok(Unit { name, path })
        }
        Kind::Process | Kind::External | Kind::Unsupported => Ok(Unit {
            name: id,
            path: PathBuf::new(),
        }),
    }
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::Config("HOME is unset — cannot place the sync daemon unit".into()))
}

/// Write `unit`'s file for `exe`/`canonical`; a no-op outside launchd/systemd.
///
/// # Errors
/// The unit directory cannot be created or the file written.
pub fn write(kind: Kind, unit: &Unit, exe: &Path, canonical: &Path) -> Result<()> {
    match kind {
        Kind::Launchd => {
            if let Some(parent) = unit.path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(
                &unit.path,
                render_launch_agent_plist(&unit.name, exe, canonical),
            )?;
            Ok(())
        }
        Kind::Systemd => {
            if let Some(parent) = unit.path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&unit.path, render_systemd_unit(exe, canonical))?;
            run_supervisor("systemctl", &["--user", "daemon-reload"]);
            Ok(())
        }
        Kind::Process | Kind::External | Kind::Unsupported => Ok(()),
    }
}

/// Start (or kick a stale) unit; a no-op for [`Kind::Process`] — the caller
/// spawns it (`super::spawn`).
///
/// # Errors
/// A supervisor CLI could not be invoked ([`Kind::External`] never gets here:
/// `ensure` verifies only).
pub fn start(kind: Kind, unit: &Unit) -> Result<()> {
    match kind {
        Kind::Launchd => {
            let uid = users_uid()?;
            let domain = format!("gui/{uid}");
            let status = Command::new("launchctl")
                .args(["bootstrap", &domain, &unit.path.display().to_string()])
                .status()
                .map_err(|e| Error::Other(format!("launchctl bootstrap: {e}")))?;
            if !status.success() {
                run_supervisor(
                    "launchctl",
                    &["kickstart", "-k", &format!("{domain}/{}", unit.name)],
                );
            }
            Ok(())
        }
        Kind::Systemd => {
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
        Kind::Process | Kind::External | Kind::Unsupported => Ok(()),
    }
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
    match kind {
        Kind::Launchd => {
            if let Ok(uid) = users_uid() {
                run_supervisor(
                    "launchctl",
                    &["bootout", &format!("gui/{uid}/{LEGACY_LAUNCHD_LABEL}")],
                );
            }
            if let Ok(home) = home() {
                let plist = home
                    .join("Library/LaunchAgents")
                    .join(format!("{LEGACY_LAUNCHD_LABEL}.plist"));
                let _ = fs::remove_file(plist);
            }
        }
        Kind::Systemd => {
            run_supervisor("systemctl", &["--user", "stop", LEGACY_SYSTEMD_UNIT]);
            if let Ok(home) = home() {
                let unit = home.join(".config/systemd/user").join(LEGACY_SYSTEMD_UNIT);
                let _ = fs::remove_file(unit);
            }
        }
        Kind::Process | Kind::External | Kind::Unsupported => {}
    }
}

/// A stray coordinator record naming a live process whose executable is
/// still `comemory`, signalled only after that check.
///
/// # Errors
/// The process table cannot be consulted.
pub fn signal_stale_pid(pid: u32) -> Result<()> {
    let exe = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .map_err(|e| Error::Other(format!("ps -p {pid}: {e}")))?;
    let name = String::from_utf8_lossy(&exe.stdout);
    if name.trim().contains("comemory") {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/supervisor.rs"]
mod tests;
