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
        // The protected `auth.json` is the resident coordinator's only
        // credential; an installer shell's exported key must not become it.
        .env_remove("COMEMORY_API_KEY")
        // The coordinator reports the backend that started it, not `foreground`.
        .env("COMEMORY_DAEMON_SUPERVISOR", "process")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    close_inherited_descriptors(&mut command);
    let _child = command.spawn()?;
    Ok(())
}

/// The last non-empty line of the process-supervised coordinator's stderr
/// log, if it wrote one — how a start that never answered explains itself.
#[must_use]
pub fn last_log_line(paths: &Paths) -> Option<String> {
    let log = paths.data_dir().join("logs/sync-daemon.err.log");
    let text = fs::read_to_string(log).ok()?;
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// The coordinator outlives whoever started it, so it must not keep any
/// descriptor it merely inherited. On macOS a multithreaded parent can leak
/// another thread's not-yet-`CLOEXEC` pipe into this process; a detached
/// coordinator holding that pipe's write end would keep the parent's reader
/// from ever seeing EOF. Every inherited descriptor above stderr that `exec`
/// would keep is closed (`CLOEXEC` ones close anyway, including std's own
/// exec-error pipe, which must survive until `exec`).
///
/// Linux marks them all close-on-exec with one `close_range` call, however
/// high they are numbered. Elsewhere (or on a kernel without
/// `CLOSE_RANGE_CLOEXEC`) the child checks each number below the
/// descriptor limit — a descriptor is always numbered below the limit in
/// force when it was opened — capped at [`MAX_SWEPT_FD`].
fn close_inherited_descriptors(command: &mut Command) {
    use nix::libc::{F_GETFD, FD_CLOEXEC, close, fcntl};
    use nix::sys::resource::{Resource, getrlimit};
    use std::os::unix::process::CommandExt as _;
    // Read in the parent: only async-signal-safe calls run after `fork`.
    let limit = getrlimit(Resource::RLIMIT_NOFILE).map_or(1024, |(soft, _)| soft);
    let limit = i32::try_from(limit.clamp(3, MAX_SWEPT_FD)).unwrap_or(i32::MAX);
    // SAFETY: the closure runs in the forked child before `exec` and only
    // makes the async-signal-safe `close_range`, `fcntl(F_GETFD)` and
    // `close` calls. A descriptor that is not open makes `fcntl` return -1
    // and is skipped.
    unsafe {
        command.pre_exec(move || {
            #[cfg(target_os = "linux")]
            {
                use nix::libc::{CLOSE_RANGE_CLOEXEC, c_int, c_uint, close_range};
                let flags = c_int::try_from(CLOSE_RANGE_CLOEXEC).unwrap_or(0);
                if flags != 0 && close_range(3, c_uint::MAX, flags) == 0 {
                    return Ok(());
                }
            }
            for fd in 3..limit {
                let flags = fcntl(fd, F_GETFD);
                if flags >= 0 && flags & FD_CLOEXEC == 0 {
                    close(fd);
                }
            }
            Ok(())
        });
    }
}

/// The highest descriptor number the portable sweep checks (1 Mi): an
/// unlimited `RLIMIT_NOFILE` must not turn `spawn` into billions of
/// syscalls.
const MAX_SWEPT_FD: u64 = 1 << 20;

#[cfg(test)]
#[path = "tests/spawn.rs"]
mod tests;
