//! `comemory sync daemon run`: the one resident coordinator for a canonical
//! data directory.
//!
//! It takes `daemon.lock` (a second one exits 75), binds the control socket
//! over any leftover, records itself in `daemon.json`, and answers `status`
//! before it reads a credential or touches the network. Then it runs the
//! pass worker, the channel thread, the reconciliation ticker and the
//! watchdog until SIGTERM, SIGINT, a `shutdown` op or its data directory
//! disappearing, or an unexpected background-thread exit; SIGHUP reloads.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Notify, mpsc, watch as generation};

use crate::config::Paths;
use crate::domains::sync::daemon::control::PROTOCOL;
use crate::domains::sync::daemon::readiness::{Readiness, SyncView};
use crate::domains::sync::daemon::server::{self, Ctx};
use crate::domains::sync::daemon::state::{State, auth_view};
use crate::domains::sync::daemon::worker::{self, Queue};
use crate::domains::sync::daemon::{
    channel, handshake, identity, runtime_record, socket_path, watchdog,
};
use crate::domains::sync::drain::{network, stop};
use crate::prelude::*;
use crate::store::random_id::random_hex;
use crate::utilities::file_lock::FileLock;

/// Lock file held for the coordinator's lifetime.
pub const DAEMON_LOCK: &str = "daemon.lock";

/// How long a stopping coordinator waits for its pass to reach a boundary.
const STOP_GRACE: Duration = Duration::from_secs(15);

/// Run the coordinator in the foreground until it is told to stop.
///
/// # Errors
/// [`Error::Conflict`] when another coordinator holds `daemon.lock`; any
/// failure to lock, bind or record before it starts answering; or
/// [`Error::Unavailable`] when a background thread exits unexpectedly.
pub async fn run(paths: &Paths) -> Result<()> {
    std::fs::create_dir_all(paths.data_dir())?;
    let canonical = identity::canonical_data_dir(paths)?;
    let paths = Paths::new(&canonical);
    let lock = FileLock::try_acquire(&canonical.join(DAEMON_LOCK), "daemon")?.ok_or_else(|| {
        let owner =
            runtime_record::read(&paths).map_or_else(String::new, |r| format!(" (pid {})", r.pid));
        Error::Conflict(format!(
            "the sync daemon is already running for {}{owner}",
            canonical.display()
        ))
    })?;
    let socket = socket_path::plan(&canonical)?;
    let listener = watchdog::bind(&socket)?;
    let state = State::new(initial_readiness(&paths, &canonical, &socket)?);
    let me = state.readiness();
    runtime_record::write(&paths, &record_of(&me))?;
    tracing::info!(pid = me.pid, socket = %socket.display(), "sync daemon answering");

    let tasks = spawn_tasks(&paths, &canonical, &socket, &state, listener)?;
    let outcome = wait_for_stop(&paths, &state, &tasks).await;
    shutdown(&state, tasks).await;
    // Release `daemon.lock` before the socket and record disappear: a
    // replacement spawned the instant this instance looks stopped must
    // never race this instance for the lock (D2/D13).
    drop(lock);
    watchdog::unbind(&socket, &me.instance, &paths);
    runtime_record::remove_if_ours(&paths, &me.instance)?;
    tracing::info!("sync daemon stopped");
    outcome
}

/// Every background handle a running coordinator holds.
struct Tasks {
    worker: std::thread::JoinHandle<()>,
    channel: std::thread::JoinHandle<()>,
    queue: Arc<Queue>,
    shutdown: Arc<Notify>,
    reload: Arc<Notify>,
    gen_tx: generation::Sender<u64>,
}

/// Start the pass worker, the channel thread, the control server and the
/// watchdog tasks.
fn spawn_tasks(
    paths: &Paths,
    canonical: &Path,
    socket: &Path,
    state: &Arc<State>,
    listener: tokio::net::UnixListener,
) -> Result<Tasks> {
    let queue = Queue::new();
    let (gen_tx, gen_rx) = generation::channel(0_u64);
    let worker = worker::spawn(paths.clone(), Arc::clone(state), Arc::clone(&queue))?;
    let channel = channel::spawn(paths.clone(), Arc::clone(state), Arc::clone(&queue), gen_rx)?;
    let (shutdown, reload) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let (rebind_tx, rebind_rx) = mpsc::channel(1);
    let ctx = Arc::new(Ctx {
        token: handshake::load_or_create(paths)?,
        uid: socket_path::owner_uid(canonical)?,
        state: Arc::clone(state),
        queue: Arc::clone(&queue),
        shutdown: Arc::clone(&shutdown),
        reload: Arc::clone(&reload),
    });
    tokio::spawn(server::serve(listener, ctx, rebind_rx));
    tokio::spawn(watchdog::tick(
        paths.clone(),
        Arc::clone(state),
        Arc::clone(&queue),
        Arc::clone(&reload),
    ));
    tokio::spawn(watchdog::guard(
        paths.clone(),
        socket.to_path_buf(),
        rebind_tx,
        Arc::clone(&shutdown),
    ));
    Ok(Tasks {
        worker,
        channel,
        queue,
        shutdown,
        reload,
        gen_tx,
    })
}

/// Stop taking new work and wait for the running pass to reach its
/// boundary. The caller releases `daemon.lock` and cleans up the socket and
/// record afterward, in that order (D2/D13).
async fn shutdown(state: &State, tasks: Tasks) {
    tracing::info!("sync daemon stopping");
    state.set_stopping();
    stop::request_shutdown();
    tasks.queue.stop();
    let _ = tasks.gen_tx.send(channel::STOP);
    let deadline = Instant::now() + STOP_GRACE;
    while (!tasks.worker.is_finished() || !tasks.channel.is_finished()) && Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tracing::debug!(
        worker_finished = tasks.worker.is_finished(),
        channel_finished = tasks.channel.is_finished(),
        "sync daemon stop grace elapsed"
    );
    join_finished(tasks.worker, "pass worker");
    join_finished(tasks.channel, "workspace channel");
}

/// Join only completed threads, preserving the shutdown deadline.
fn join_finished(thread: std::thread::JoinHandle<()>, name: &str) {
    if thread.is_finished() && thread.join().is_err() {
        tracing::error!(thread = name, "sync daemon background thread panicked");
    }
}

/// Serve reloads until a stop or an unexpected background-thread exit.
async fn wait_for_stop(paths: &Paths, state: &State, tasks: &Tasks) -> Result<()> {
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    let mut hup = signal(SignalKind::hangup())?;
    let mut health = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            () = tasks.shutdown.notified() => return Ok(()),
            _ = term.recv() => return Ok(()),
            _ = int.recv() => return Ok(()),
            _ = hup.recv() => apply_reload(paths, state, &tasks.gen_tx),
            () = tasks.reload.notified() => apply_reload(paths, state, &tasks.gen_tx),
            _ = health.tick() => {
                for (name, thread) in [("pass worker", &tasks.worker), ("workspace channel", &tasks.channel)] {
                    if thread.is_finished() {
                        return Err(Error::Unavailable(format!("sync daemon {name} exited unexpectedly")));
                    }
                }
            }
        }
    }
}

/// Re-read the credential and restart the channel under it.
pub fn apply_reload(paths: &Paths, state: &State, generation: &generation::Sender<u64>) {
    state.refresh_auth(paths);
    generation.send_modify(|g| *g = g.wrapping_add(1) % channel::STOP);
}

fn initial_readiness(paths: &Paths, canonical: &Path, socket: &Path) -> Result<Readiness> {
    let me = identity::BinaryIdentity::current()?;
    Ok(Readiness {
        protocol: PROTOCOL,
        version: me.version,
        binary: me.path,
        binary_file: me.file,
        pid: std::process::id(),
        instance: random_hex(8)?,
        started_at: network::now()?,
        data_dir: canonical.to_path_buf(),
        socket: socket.to_path_buf(),
        supervisor: crate::config::env::daemon_supervisor_override()
            .unwrap_or_else(|| "foreground".into()),
        store: crate::store::readiness::probe(&paths.db_path()).map_or(
            crate::domains::sync::daemon::readiness::StoreState::Error,
            Into::into,
        ),
        auth: auth_view(paths),
        sync: SyncView {
            last_verify_at: worker::last_verify(paths).map(|(_, at)| at),
            ..SyncView::default()
        },
    })
}

fn record_of(r: &Readiness) -> runtime_record::RuntimeRecord {
    runtime_record::RuntimeRecord {
        pid: r.pid,
        instance: r.instance.clone(),
        socket: r.socket.clone(),
        version: r.version.clone(),
        binary: r.binary.clone(),
        started_at: r.started_at.clone(),
        supervisor: r.supervisor.clone(),
    }
}

#[cfg(test)]
#[path = "tests/coordinator.rs"]
mod tests;
