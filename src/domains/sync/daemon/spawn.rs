//! `Kind::Process` supervision: this build starts the coordinator itself as
//! a detached child, for hosts with no usable launchd/systemd.

use std::fs;
use std::process::{Command, Stdio};

use crate::config::Paths;
use crate::domains::sync::daemon::identity::BinaryIdentity;
use crate::prelude::*;

/// Spawn `<exe> --data-dir <canonical> sync daemon run`, detached from this
/// process's session, stdio to `logs/sync-daemon.{out,err}.log`.
///
/// # Errors
/// The log directory or the child cannot be created.
pub fn spawn(paths: &Paths) -> Result<()> {
    let canonical = crate::domains::sync::daemon::identity::canonical_data_dir(paths)?;
    let me = BinaryIdentity::current()?;
    let log_dir = canonical.join("logs");
    fs::create_dir_all(&log_dir)?;
    let stdout = fs::File::create(log_dir.join("sync-daemon.out.log"))?;
    let stderr = fs::File::create(log_dir.join("sync-daemon.err.log"))?;
    let mut command = Command::new(&me.path);
    command
        .arg("--data-dir")
        .arg(&canonical)
        .args(["sync", "daemon", "run", "--detach-session"])
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    let _child = command.spawn()?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/spawn.rs"]
mod tests;
