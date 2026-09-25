//! The pass worker: one plain thread (so `reqwest::blocking` is safe) that
//! takes the whole pending set at once — every wake since the last pass is
//! one pass — and runs it under `sync.lock`, so it never overlaps a manual
//! sync, an inline push or a hook's in-process pass.
//!
//! A pass is the `sync --action auto` pass ([`auto::run_pass`]) plus a verify
//! when `[sync] verify_every` has elapsed since `sync-verify.last`. It never
//! opens a database that is absent, behind or ahead ([`store::readiness`]).
//!
//! [`store::readiness`]: crate::store::readiness

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use tokio::sync::oneshot;

use crate::config::{Config, Paths};
use crate::domains::code::hooked_refresh::{self, RefreshStats};
use crate::domains::sync::daemon::readiness::{PassSummary, StoreState, Trigger};
use crate::domains::sync::daemon::state::State;
use crate::domains::sync::drain::network;
use crate::domains::sync::{AuthFile, auto, verify};
use crate::prelude::*;
use crate::store::{connection, readiness};

/// File whose mtime is the last successful verify.
pub const VERIFY_STAMP: &str = "sync-verify.last";

/// Work waiting for the next pass.
#[derive(Default)]
struct Pending {
    trigger: Option<Trigger>,
    checkouts: Vec<PathBuf>,
    waiters: Vec<oneshot::Sender<PassSummary>>,
    stopping: bool,
}

/// The pending set and the signal that it changed.
#[derive(Default)]
pub struct Queue {
    pending: Mutex<Pending>,
    ready: Condvar,
}

/// One pass's worth of work, taken whole.
struct Work {
    trigger: Trigger,
    checkouts: Vec<PathBuf>,
    waiters: Vec<oneshot::Sender<PassSummary>>,
}

impl Queue {
    /// An empty queue.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, Pending> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Merge a wake into the pending set.
    pub fn wake(&self, trigger: Trigger, checkout: Option<PathBuf>) {
        let mut p = self.lock();
        p.trigger.get_or_insert(trigger);
        if let Some(path) = checkout.filter(|c| !p.checkouts.contains(c)) {
            p.checkouts.push(path);
        }
        self.ready.notify_all();
    }

    /// Queue a pass and hand back the receiver of its summary.
    #[must_use]
    pub fn catch_up(&self) -> oneshot::Receiver<PassSummary> {
        let (tx, rx) = oneshot::channel();
        let mut p = self.lock();
        p.trigger.get_or_insert(Trigger::CatchUp);
        p.waiters.push(tx);
        self.ready.notify_all();
        rx
    }

    /// Whether work waits.
    #[must_use]
    pub fn is_queued(&self) -> bool {
        self.lock().trigger.is_some()
    }

    /// Stop after the running pass; pending work is dropped.
    pub fn stop(&self) {
        self.lock().stopping = true;
        self.ready.notify_all();
    }

    /// Block until there is work (or `None` once stopping).
    fn take(&self) -> Option<Work> {
        let mut p = self.lock();
        loop {
            if p.stopping {
                return None;
            }
            if let Some(trigger) = p.trigger.take() {
                return Some(Work {
                    trigger,
                    checkouts: std::mem::take(&mut p.checkouts),
                    waiters: std::mem::take(&mut p.waiters),
                });
            }
            p = self
                .ready
                .wait(p)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

/// Start the worker thread.
///
/// # Errors
/// The thread cannot be spawned.
pub fn spawn(paths: Paths, state: Arc<State>, queue: Arc<Queue>) -> Result<JoinHandle<()>> {
    Ok(std::thread::Builder::new()
        .name("comemory-sync-pass".into())
        .spawn(move || {
            while let Some(work) = queue.take() {
                state.set_queued(queue.is_queued());
                state.pass_started(work.trigger);
                let summary = pass(&paths, work.trigger, &work.checkouts);
                if let Some(at) = last_verify(&paths).map(|(_, at)| at) {
                    state.verified(at);
                }
                state.set_queued(queue.is_queued());
                state.pass_finished(&summary);
                for waiter in work.waiters {
                    let _ = waiter.send(summary.clone());
                }
            }
        })?)
}

/// Run one pass; every failure lands in the summary, never in a panic.
#[must_use]
pub fn pass(paths: &Paths, trigger: Trigger, checkouts: &[PathBuf]) -> PassSummary {
    let mut summary = PassSummary {
        trigger,
        finished_at: String::new(),
        logged_in: false,
        end: None,
        pulled: 0,
        pushed: 0,
        store: StoreState::Error,
        error: None,
    };
    if let Err(e) = run(paths, checkouts, &mut summary) {
        tracing::warn!(error = %e, "sync daemon pass failed");
        summary.error.get_or_insert(e.to_string());
    }
    summary.finished_at = network::now().unwrap_or_default();
    summary
}

fn run(paths: &Paths, checkouts: &[PathBuf], summary: &mut PassSummary) -> Result<()> {
    let cfg = load_config(paths)?;
    let _pass = auto::hold_pass_lock(paths)?;
    summary.store = readiness::probe(&paths.db_path())?.into();
    if summary.store != StoreState::Ready {
        return Ok(());
    }
    let mut conn = connection::open(paths.db_path())?;
    let mut refresh = RefreshStats::default();
    let mut resolved = Vec::new();
    for path in checkouts {
        match hooked_refresh::resolve_checkout(path) {
            Some(checkout) => resolved.push(checkout),
            None => {
                refresh.record_failure(&path.display().to_string(), "not inside a git work tree");
            }
        }
    }
    for extra in resolved.iter().skip(1) {
        hooked_refresh::index_checkout(paths, &cfg, &mut conn, extra, &mut refresh)?;
    }
    let stats = auto::run_pass(paths, &cfg, &mut conn, resolved.first(), refresh)?;
    summary.logged_in = stats.logged_in;
    summary.error = stats.error;
    if let Some(exchange) = &stats.run.exchange {
        summary.end = Some(exchange.end);
        summary.pulled += exchange.pulled;
        summary.pushed += exchange.pushed;
    }
    summary.pulled += stats.run.pull.as_ref().map_or(0, |p| p.pulled);
    summary.pushed += stats.run.push.as_ref().map_or(0, |p| p.pushed);
    if stats.logged_in {
        verify_if_due(paths, &cfg, &mut conn)?;
    }
    Ok(())
}

/// Verify when `[sync] verify_every` has elapsed since the stamp.
fn verify_if_due(paths: &Paths, cfg: &Config, conn: &mut crate::store::Connection) -> Result<()> {
    let every = cfg.sync.verify_every_duration()?;
    let due = last_verify(paths).is_none_or(|(at, _)| {
        SystemTime::now()
            .duration_since(at)
            .unwrap_or(Duration::ZERO)
            >= every
    });
    let Some(auth) = AuthFile::load_usable(paths)?.filter(|_| due) else {
        return Ok(());
    };
    let report = verify::verify(paths, cfg, conn, &auth)?;
    tracing::info!(?report, "sync daemon verify finished");
    std::fs::write(paths.data_dir().join(VERIFY_STAMP), network::now()?)?;
    Ok(())
}

/// The last successful verify: the stamp's mtime and its recorded time.
#[must_use]
pub fn last_verify(paths: &Paths) -> Option<(SystemTime, String)> {
    let stamp = paths.data_dir().join(VERIFY_STAMP);
    let at = std::fs::metadata(&stamp).ok()?.modified().ok()?;
    let text = std::fs::read_to_string(&stamp).ok()?;
    Some((at, text.trim().to_string()))
}

/// Config the way the CLI loads it: defaults, file, environment.
///
/// # Errors
/// An unreadable or invalid `config.toml`.
pub fn load_config(paths: &Paths) -> Result<Config> {
    Config::defaults()
        .with_file(paths.config_file().as_path())?
        .with_env()
}
