//! Shared `launchctl`/`systemctl` plumbing for [`super::daemon::supervisor`],
//! the per-directory unit manager (#257). The pre-#257 single, un-id'd unit
//! this module used to own is gone; [`super::daemon::supervisor::remove_legacy`]
//! retires any leftover from an older build.

use std::process::Command;

use crate::prelude::*;

/// Run a supervisor CLI and warn on spawn/non-zero exit (best-effort path).
pub(crate) fn run_supervisor(program: &str, args: &[&str]) {
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

/// This user's numeric uid, via `id -u`; refuses uid 0 (the root GUI domain).
#[cfg(target_os = "macos")]
pub(crate) fn users_uid() -> Result<u32> {
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
