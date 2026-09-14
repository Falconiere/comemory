//! LaunchAgent / systemd unit install, start, stop, and status probes.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

use crate::config::Paths;
use crate::prelude::*;

/// launchd label / systemd unit stem.
pub const DAEMON_LABEL: &str = "io.comemory.sync";

/// systemd `--user` unit file name.
pub const SYSTEMD_UNIT: &str = "comemory-sync.service";

pub use crate::sync::daemon_templates::{render_launch_agent_plist, render_systemd_unit};

/// Installed / running report for status surfaces.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DaemonStatus {
    /// Platform this build understands (`macos`, `linux`, or `unsupported`).
    pub platform: &'static str,
    /// Unit / plist path when known.
    pub unit_path: Option<String>,
    /// Whether the unit file exists on disk.
    pub installed: bool,
    /// Whether the supervisor reports the job as running.
    pub running: bool,
    /// One-line human detail (e.g. auto-sync inactive warning).
    pub detail: String,
}

impl DaemonStatus {
    /// Warning when linked but the daemon is not running.
    pub fn inactive_warning(&self) -> Option<&'static str> {
        if self.running {
            None
        } else {
            Some("auto-sync inactive — run `comemory sync daemon start` (or re-login)")
        }
    }
}

fn current_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
}

/// Path to the LaunchAgent plist (macOS).
pub fn launch_agent_plist() -> Result<PathBuf> {
    Ok(dirs_home()?
        .join("Library/LaunchAgents")
        .join(format!("{DAEMON_LABEL}.plist")))
}

/// Path to the systemd user unit (Linux).
pub fn systemd_user_unit() -> Result<PathBuf> {
    Ok(dirs_home()?.join(".config/systemd/user").join(SYSTEMD_UNIT))
}

fn dirs_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::Config("HOME is unset — cannot place the sync daemon unit".into()))
}

/// Run a supervisor CLI and warn on spawn/non-zero exit (best-effort path).
fn run_supervisor(program: &str, args: &[&str]) {
    match Command::new(program).args(args).status() {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::warn!(
            program,
            ?args,
            code = ?status.code(),
            "supervisor command exited non-zero"
        ),
        Err(e) => tracing::warn!(program, ?args, error = %e, "supervisor command failed to spawn"),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn unsupported() -> Error {
    Error::Usage("comemory sync daemon is not supported on this OS (macOS and Linux only)".into())
}

/// Write the unit file (does not start it).
pub fn install(paths: &Paths) -> Result<PathBuf> {
    let exe = current_binary()?;
    let data_dir = paths.data_dir();
    fs::create_dir_all(data_dir.join("logs"))?;

    #[cfg(target_os = "macos")]
    {
        let plist = launch_agent_plist()?;
        if let Some(parent) = plist.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&plist, render_launch_agent_plist(&exe, data_dir))?;
        Ok(plist)
    }
    #[cfg(target_os = "linux")]
    {
        let unit = systemd_user_unit()?;
        if let Some(parent) = unit.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&unit, render_systemd_unit(&exe, data_dir))?;
        run_supervisor("systemctl", &["--user", "daemon-reload"]);
        run_supervisor("systemctl", &["--user", "enable", SYSTEMD_UNIT]);
        Ok(unit)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (exe, data_dir);
        Err(unsupported())
    }
}

/// Remove the unit file and unload/disable it.
pub fn uninstall() -> Result<()> {
    stop();
    #[cfg(target_os = "macos")]
    {
        let plist = launch_agent_plist()?;
        if plist.exists() {
            fs::remove_file(&plist)?;
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        run_supervisor("systemctl", &["--user", "disable", "--now", SYSTEMD_UNIT]);
        let unit = systemd_user_unit()?;
        if unit.exists() {
            fs::remove_file(&unit)?;
        }
        run_supervisor("systemctl", &["--user", "daemon-reload"]);
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(unsupported())
    }
}

/// Load/start the installed unit.
pub fn start() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let plist = launch_agent_plist()?;
        if !plist.exists() {
            return Err(Error::Usage(
                "sync daemon is not installed — run `comemory sync daemon install`".into(),
            ));
        }
        let uid = users_uid()?;
        let domain = format!("gui/{uid}");
        let status = Command::new("launchctl")
            .args(["bootstrap", &domain, &plist.display().to_string()])
            .status()
            .map_err(|e| Error::Other(format!("launchctl bootstrap: {e}")))?;
        if !status.success() {
            tracing::warn!(
                code = ?status.code(),
                "launchctl bootstrap failed; trying kickstart of an already-loaded job"
            );
            run_supervisor(
                "launchctl",
                &["kickstart", "-k", &format!("{domain}/{DAEMON_LABEL}")],
            );
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let unit = systemd_user_unit()?;
        if !unit.exists() {
            return Err(Error::Usage(
                "sync daemon is not installed — run `comemory sync daemon install`".into(),
            ));
        }
        let status = Command::new("systemctl")
            .args(["--user", "start", SYSTEMD_UNIT])
            .status()
            .map_err(|e| Error::Other(format!("systemctl start: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::Other(format!(
                "systemctl --user start {SYSTEMD_UNIT} failed"
            )))
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(unsupported())
    }
}

/// Stop the daemon without removing the unit.
pub fn stop() {
    #[cfg(target_os = "macos")]
    {
        match users_uid() {
            Ok(uid) => {
                let target = format!("gui/{uid}/{DAEMON_LABEL}");
                run_supervisor("launchctl", &["bootout", &target]);
            }
            Err(e) => tracing::warn!(error = %e, "sync daemon stop skipped"),
        }
    }
    #[cfg(target_os = "linux")]
    {
        run_supervisor("systemctl", &["--user", "stop", SYSTEMD_UNIT]);
    }
}

/// Report whether the unit is installed and running.
pub fn status() -> Result<DaemonStatus> {
    #[cfg(target_os = "macos")]
    {
        let plist = launch_agent_plist()?;
        let installed = plist.exists();
        let running = installed && launchd_running();
        Ok(DaemonStatus {
            platform: "macos",
            unit_path: Some(plist.display().to_string()),
            installed,
            running,
            detail: status_detail(installed, running),
        })
    }
    #[cfg(target_os = "linux")]
    {
        let unit = systemd_user_unit()?;
        let installed = unit.exists();
        let running = installed && systemd_running();
        Ok(DaemonStatus {
            platform: "linux",
            unit_path: Some(unit.display().to_string()),
            installed,
            running,
            detail: status_detail(installed, running),
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Ok(DaemonStatus {
            platform: "unsupported",
            unit_path: None,
            installed: false,
            running: false,
            detail: "sync daemon is not supported on this OS".into(),
        })
    }
}

fn status_detail(installed: bool, running: bool) -> String {
    match (installed, running) {
        (true, true) => "running".into(),
        (true, false) => "installed but not running — auto-sync inactive".into(),
        (false, _) => "not installed — auto-sync inactive".into(),
    }
}

#[cfg(target_os = "macos")]
fn launchd_running() -> bool {
    let Ok(uid) = users_uid() else {
        return false;
    };
    let target = format!("gui/{uid}/{DAEMON_LABEL}");
    let Ok(out) = Command::new("launchctl").args(["print", &target]).output() else {
        return false;
    };
    out.status.success() && String::from_utf8_lossy(&out.stdout).contains("state = running")
}

#[cfg(target_os = "linux")]
fn systemd_running() -> bool {
    let Ok(out) = Command::new("systemctl")
        .args(["--user", "is-active", SYSTEMD_UNIT])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).trim() == "active"
}

#[cfg(target_os = "macos")]
fn users_uid() -> Result<u32> {
    let out = Command::new("id")
        .arg("-u")
        .output()
        .map_err(|e| Error::Other(format!("id -u: {e}")))?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "id -u exited {}",
            out.status.code().unwrap_or(-1)
        )));
    }
    let raw =
        String::from_utf8(out.stdout).map_err(|e| Error::Other(format!("id -u stdout: {e}")))?;
    let uid: u32 = raw
        .trim()
        .parse()
        .map_err(|_| Error::Other(format!("id -u returned non-numeric uid: {raw:?}")))?;
    if uid == 0 {
        return Err(Error::Other(
            "refusing to manage the sync daemon in the root GUI domain (uid 0)".into(),
        ));
    }
    Ok(uid)
}
