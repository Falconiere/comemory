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
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::{Paths, env};
use crate::domains::sync::daemon::identity::{UNIT_ID_LEN, data_dir_id};
use crate::domains::sync::daemon_templates::{render_launch_agent_plist, render_systemd_unit};
use crate::domains::sync::daemon_unit::{run_supervisor, users_uid};
use crate::prelude::*;

/// A legacy (un-id'd) unit this issue's per-directory scheme retires.
const LEGACY_LAUNCHD_LABEL: &str = "io.comemory.sync";
/// A legacy (un-id'd) systemd unit this issue's per-directory scheme retires.
const LEGACY_SYSTEMD_UNIT: &str = "comemory-sync.service";

/// How long launchd may take to finish removing a booted-out job: it SIGKILLs
/// a process that ignores SIGTERM only after its default 20 s `ExitTimeOut`.
const BOOTOUT_BOUND: Duration = Duration::from_secs(25);

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

/// [`detect`], narrowed by [`for_data_dir`] for the directory `paths` names:
/// the backend that really supervises its coordinator. Starting, reporting
/// and `status` all take this one answer, so none of them names a
/// LaunchAgent that was never written.
///
/// # Errors
/// [`Error::Config`] for an unrecognized override value.
pub fn detect_for(paths: &Paths) -> Result<Kind> {
    let kind = detect()?;
    if !can_narrow(kind) {
        return Ok(kind);
    }
    Ok(for_data_dir(kind, &resolve(paths.data_dir())))
}

/// The backend that may supervise `canonical` when `kind` was auto-detected.
/// A data directory under the OS temp directory is throwaway (a test run, a
/// scratch probe): a LaunchAgent or systemd unit for it would outlive the
/// run, pile up in the user's login items and keep a coordinator resident for
/// a directory nobody returns to, so it is supervised as a plain process.
/// An explicit `COMEMORY_DAEMON_SUPERVISOR` is always honored, and so is a
/// directory inside the user's home.
#[must_use]
pub fn for_data_dir(kind: Kind, canonical: &Path) -> Kind {
    if can_narrow(kind) && is_ephemeral(canonical) {
        Kind::Process
    } else {
        kind
    }
}

/// Whether `kind` is a native backend the user did not ask for by name.
fn can_narrow(kind: Kind) -> bool {
    matches!(kind, Kind::Launchd | Kind::Systemd) && env::daemon_supervisor_override().is_none()
}

/// `path` with symlinks resolved as far as it exists: a directory not yet
/// created still resolves through its nearest existing ancestor, so `status`
/// (which never creates it) and `ensure` (which does) judge the same path.
fn resolve(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut resolved = PathBuf::new();
    let mut missing = Vec::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if missing.pop().is_none() {
                    resolved.pop();
                }
            }
            Component::Normal(name) if missing.is_empty() => {
                match fs::canonicalize(resolved.join(name)) {
                    Ok(canonical) => resolved = canonical,
                    Err(_) => missing.push(name.to_os_string()),
                }
            }
            Component::Normal(name) => missing.push(name.to_os_string()),
        }
    }
    missing
        .into_iter()
        .fold(resolved, |path, component| path.join(component))
}

/// Throwaway roots, in the canonical form a resolved data directory has:
/// macOS resolves `/tmp` and `/var/tmp` under `/private` and keeps `$TMPDIR`
/// under `/private/var/folders`; Linux keeps them as written.
const EPHEMERAL_ROOTS: [&str; 5] = [
    "/tmp",
    "/private/tmp",
    "/var/tmp",
    "/private/var/tmp",
    "/private/var/folders",
];

/// Whether `canonical` lives under a temp root. A directory inside the user's
/// home never does, however the home and the temp root overlap (`HOME=/tmp/u`,
/// `TMPDIR=$HOME`).
fn is_ephemeral(canonical: &Path) -> bool {
    if home().is_ok_and(|home| canonical.starts_with(resolve(&home))) {
        return false;
    }
    EPHEMERAL_ROOTS
        .iter()
        .any(|root| canonical.starts_with(root))
        || fs::canonicalize(std::env::temp_dir()).is_ok_and(|temp| canonical.starts_with(temp))
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
    bootstrap_or_replace(unit, &domain)
}

fn bootstrap_or_replace(unit: &Unit, domain: &str) -> Result<()> {
    if !bootstrap_launchd(unit, domain)? {
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
        // `bootout` returns before launchd has finished removing the job — it
        // may still be stopping the job's process. Bootstrapping the same
        // label in that window fails with "5: Input/output error", and the
        // process fallback then loses `daemon.lock` to the dying coordinator
        // (homebrew-tap#1, F-4). Wait until the label is gone.
        wait_until_unloaded(domain, &unit.name, BOOTOUT_BOUND)?;
        if !bootstrap_launchd(unit, domain)? {
            return Err(Error::Other(format!(
                "launchctl bootstrap {} failed",
                unit.name
            )));
        }
    }
    Ok(())
}

/// Poll `launchctl print <domain>/<label>` until it fails (the job is gone),
/// for at most `bound`.
fn wait_until_unloaded(domain: &str, label: &str, bound: Duration) -> Result<()> {
    let deadline = Instant::now() + bound;
    while launchd_loaded(domain, label)? {
        if Instant::now() >= deadline {
            return Err(Error::Other(format!(
                "launchctl bootout {label}: still loaded after {}s",
                bound.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// Whether launchd still has `label` in `domain`.
fn launchd_loaded(domain: &str, label: &str) -> Result<bool> {
    launchctl(&["print", &format!("{domain}/{label}")], Stdio::null)
}

fn bootstrap_launchd(unit: &Unit, domain: &str) -> Result<bool> {
    launchctl(
        &["bootstrap", domain, &unit.path.display().to_string()],
        Stdio::inherit,
    )
}

/// Run `launchctl <args>` and report whether it succeeded. `output` decides
/// where its stdout/stderr go: `print` is only a probe, while a failed
/// `bootstrap` explains itself on the caller's stderr.
fn launchctl(args: &[&str], output: fn() -> Stdio) -> Result<bool> {
    Command::new("launchctl")
        .args(args)
        .stdout(output())
        .stderr(output())
        .status()
        .map(|status| status.success())
        .map_err(|e| Error::Other(format!("launchctl {}: {e}", args.join(" "))))
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

/// Boot out and remove `canonical`'s unit under `kind` if one was written;
/// best-effort, and a no-op when there is none.
pub fn retire_unit(kind: Kind, canonical: &Path) {
    if let Ok(unit) = plan(canonical, kind)
        && unit.path.exists()
    {
        remove(kind, &unit);
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
