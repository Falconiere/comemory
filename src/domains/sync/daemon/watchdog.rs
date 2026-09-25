//! The coordinator's timers: binding the socket over a leftover, the
//! reconciliation ticker, and the guard that re-binds a removed or replaced
//! socket and stops the coordinator when its data directory disappears.

use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UnixListener;
use tokio::sync::{Notify, mpsc};

use crate::config::Paths;
use crate::domains::sync::daemon::readiness::Trigger;
use crate::domains::sync::daemon::state::State;
use crate::domains::sync::daemon::worker::{self, Queue};
use crate::prelude::*;

/// The shortest reconciliation interval honored.
const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// How often the guard looks at the socket and the data directory.
const GUARD_EVERY: Duration = Duration::from_secs(1);

/// Bind `socket`, replacing whatever a dead predecessor left there. Only a
/// caller holding `daemon.lock` may call this: the path is then ours.
///
/// # Errors
/// A directory sits at the path, or binding fails.
pub fn bind(socket: &Path) -> Result<UnixListener> {
    match std::fs::symlink_metadata(socket) {
        Ok(meta) if meta.is_dir() => {
            return Err(Error::Unavailable(format!(
                "{} is a directory — remove it so the sync daemon can bind",
                socket.display()
            )));
        }
        Ok(_) => std::fs::remove_file(socket)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener = std::os::unix::net::UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(UnixListener::from_std(listener)?)
}

/// Remove our socket on the way out; a leftover would only cost a probe.
pub fn unbind(socket: &Path, instance: &str, paths: &Paths) {
    let ours = std::fs::symlink_metadata(socket).is_ok_and(|m| m.file_type().is_socket());
    if ours && let Err(e) = std::fs::remove_file(socket) {
        tracing::debug!(error = %e, instance, data_dir = %paths.data_dir().display(), "socket removal failed");
    }
}

/// Queue a reconciliation pass now and every `[sync] daemon_interval`,
/// re-reading the config each time; a credential that changed on disk
/// raises `reload`.
pub async fn tick(paths: Paths, state: Arc<State>, queue: Arc<Queue>, reload: Arc<Notify>) {
    let mut interval = Duration::from_secs(5);
    loop {
        match worker::load_config(&paths).and_then(|c| c.sync.daemon_interval_duration()) {
            Ok(configured) => interval = configured.max(MIN_INTERVAL),
            Err(e) => {
                tracing::warn!(error = %e, "sync daemon kept its interval: config unreadable")
            }
        }
        state.set_interval(interval.as_secs());
        if state.refresh_auth(&paths) {
            reload.notify_one();
        }
        queue.wake(Trigger::Tick, None);
        state.set_queued(true);
        tokio::time::sleep(interval).await;
    }
}

/// Watch the socket and the data directory until the directory is gone.
pub async fn guard(
    paths: Paths,
    socket: PathBuf,
    rebind: mpsc::Sender<UnixListener>,
    shutdown: Arc<Notify>,
) {
    let mut bound = inode(&socket);
    loop {
        tokio::time::sleep(GUARD_EVERY).await;
        if !paths.data_dir().exists() {
            tracing::info!("sync daemon data directory removed; stopping");
            shutdown.notify_one();
            return;
        }
        if bound.is_some() && inode(&socket) == bound {
            continue;
        }
        match bind(&socket) {
            Ok(listener) => {
                tracing::info!(socket = %socket.display(), "sync daemon re-bound its socket");
                bound = inode(&socket);
                if rebind.send(listener).await.is_err() {
                    return;
                }
            }
            Err(e) => tracing::warn!(error = %e, "sync daemon could not re-bind its socket"),
        }
    }
}

/// The socket's inode, when a socket is there.
fn inode(socket: &Path) -> Option<u64> {
    std::fs::symlink_metadata(socket)
        .ok()
        .filter(|m| m.file_type().is_socket())
        .map(|m| m.ino())
}
