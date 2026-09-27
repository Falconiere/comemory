//! `ensure`/`restart`/`repair`/preflight: verify the coordinator for a data
//! directory, and — unless the caller only wants a read — repair it.
//!
//! `daemon-ensure.lock` serializes repairs: the first caller to acquire it
//! does the work; everyone else re-probes once it is free, so eight
//! concurrent callers start at most one coordinator (A-2).

use std::time::{Duration, Instant};

use crate::config::Paths;
use crate::domains::sync::daemon::client::{self, Probe};
use crate::domains::sync::daemon::readiness::Readiness;
use crate::domains::sync::daemon::{identity, spawn, supervisor};
use crate::prelude::*;
use crate::utilities::file_lock::FileLock;

/// Lock file serializing repairs for one data directory.
pub const ENSURE_LOCK: &str = "daemon-ensure.lock";

/// How long an evicted coordinator may take to drain and release
/// `daemon.lock`: its own 15 s stop grace plus a margin.
const EVICT_BOUND: Duration = Duration::from_secs(20);

/// How the caller intends to use the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// A verified coordinator must exist before this command proceeds.
    Preflight,
    /// `comemory sync daemon ensure`.
    Ensure,
    /// `comemory sync daemon restart`: stop first, then repair.
    Restart,
    /// `comemory sync daemon repair`: also rewrite the unit unconditionally.
    Repair,
}

impl Intent {
    /// Bound on the whole call, including any repair and readiness wait.
    const fn bound(self) -> Duration {
        match self {
            Self::Preflight => Duration::from_secs(10),
            Self::Ensure | Self::Restart | Self::Repair => Duration::from_secs(20),
        }
    }
}

/// What `ensure` (or a preflight) did.
#[derive(Debug, Clone)]
pub struct Ensured {
    /// Whether a verified coordinator answers now.
    pub ready: bool,
    /// `none`, `started`, `restarted` or `replaced`.
    pub action: &'static str,
    /// The backend used or attempted.
    pub supervisor: supervisor::Kind,
    /// Non-fatal detail (a fallback reason, a stale-owner notice).
    pub notes: Vec<String>,
    /// The verified readiness, when `ready`.
    pub daemon: Option<Readiness>,
    /// Why `ready` is false.
    pub error: Option<String>,
}

impl Ensured {
    /// The `sync daemon ensure --json` document — also what installers and
    /// `comemory upgrade` parse to verify the coordinator they started.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ready": self.ready,
            "action": self.action,
            "supervisor": self.supervisor.as_str(),
            "notes": self.notes,
            "daemon": self.daemon,
            "error": self.error,
        })
    }
}

/// Verify, and unless `intent` is a read-only preflight that already found a
/// healthy coordinator, repair the coordinator for `paths`.
///
/// # Errors
/// Configuration and filesystem failures reaching the data directory itself
/// (an unreachable coordinator is reported in the returned value, not an
/// `Err`).
pub fn ensure(paths: &Paths, intent: Intent) -> Result<Ensured> {
    std::fs::create_dir_all(paths.data_dir())?;
    let deadline = Instant::now() + intent.bound();
    let probe_bound = || remaining(deadline, client::PROBE_BOUND);

    if matches!(intent, Intent::Restart) {
        stop_if_healthy(paths, deadline);
    }

    let probe = client::probe(paths, probe_bound());
    if let Some(result) = accept(intent, probe) {
        return Ok(result);
    }

    repair(paths, intent, deadline)
}

/// Whether `probe` already satisfies `intent` without repairing.
fn accept(intent: Intent, probe: Probe) -> Option<Ensured> {
    let Probe::Healthy(readiness) = probe else {
        return None;
    };
    if matches!(intent, Intent::Repair) {
        return None;
    }
    let current = identity::BinaryIdentity::current().ok()?;
    fits(intent, &readiness, &current).then(|| Ensured {
        ready: true,
        action: "none",
        // `detect` is a pure env/OS check (no I/O), so the fast accept path
        // still reports the real backend rather than a placeholder.
        supervisor: supervisor::detect().unwrap_or(supervisor::Kind::External),
        notes: Vec::new(),
        daemon: Some(*readiness),
        error: None,
    })
}

/// Whether a verified coordinator may stay for `intent`. `ensure`,
/// `restart` and `repair` want exactly the caller's binary: version, path
/// and file (#258 D3a). Preflight keeps any coordinator whose binary still
/// exists unless [`identity::preflight_replaces`] says the file under it was swapped.
fn fits(intent: Intent, readiness: &Readiness, current: &identity::BinaryIdentity) -> bool {
    if matches!(intent, Intent::Preflight) {
        return identity::preflight_accepts(readiness, current);
    }
    readiness.version == current.version
        && readiness.binary == current.path
        && (current.file.is_none() || readiness.binary_file == current.file)
}

/// Best-effort graceful stop, waiting up to `deadline` — the caller's own
/// overall budget — for it to take. A coordinator mid-drain can cost the
/// full stop-grace bound to reach its own boundary; giving up early here
/// would start a second one racing the first for `daemon.lock`.
fn stop_if_healthy(paths: &Paths, deadline: Instant) {
    if let Probe::Healthy(_) = client::probe(paths, remaining(deadline, client::PROBE_BOUND)) {
        let _ = client::shutdown(paths, remaining(deadline, client::PROBE_BOUND));
        while Instant::now() < deadline
            && matches!(
                client::probe(paths, remaining(deadline, Duration::from_millis(200))),
                Probe::Healthy(_)
            )
        {}
        wait_for_lock_release(paths, deadline);
    }
}

/// A stopping coordinator stops answering before it drops `daemon.lock`
/// (its pass may still be reaching a boundary). A replacement started in
/// that window exits 75 on the lock, so wait until the lock is free.
fn wait_for_lock_release(paths: &Paths, deadline: Instant) {
    let lock = paths
        .data_dir()
        .join(crate::domains::sync::daemon::coordinator::DAEMON_LOCK);
    while Instant::now() < deadline {
        match FileLock::try_acquire(&lock, "daemon") {
            Ok(Some(_released)) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            // The start that follows reports the same filesystem problem.
            Err(error) => {
                tracing::debug!(%error, "daemon.lock unreadable while waiting for release");
                return;
            }
        }
    }
}

/// Serialize on [`ENSURE_LOCK`], re-probe (a racer may have just fixed it),
/// then write/start the backend `[`supervisor::detect`]` chooses.
fn repair(paths: &Paths, intent: Intent, deadline: Instant) -> Result<Ensured> {
    let lock_path = paths.data_dir().join(ENSURE_LOCK);
    let _lock = loop {
        if let Some(lock) = FileLock::try_acquire(&lock_path, "daemon-ensure")? {
            break lock;
        }
        if let Some(mut repaired) = accept(
            settled(intent),
            client::probe(paths, Duration::from_millis(200)),
        ) {
            repaired
                .notes
                .push("a concurrent ensure already repaired the coordinator".into());
            return Ok(repaired);
        }
        if Instant::now() >= deadline {
            let supervisor = supervisor::detect().unwrap_or(supervisor::Kind::Process);
            return Ok(not_ready(
                supervisor,
                Vec::new(),
                local_service_error(supervisor),
            ));
        }
    };

    repair_locked(paths, intent, deadline)
}

fn repair_locked(paths: &Paths, intent: Intent, deadline: Instant) -> Result<Ensured> {
    // The lock may have waited; a healthy coordinator can already exist.
    let probe = client::probe(paths, Duration::from_millis(200));
    let evicting = matches!(probe, Probe::Healthy(_));
    if let Some(accepted) = accept(intent, probe) {
        return Ok(accepted);
    }
    // Whatever answered has the wrong identity (or nothing does): evict it
    // gracefully before starting a fresh one, so two coordinators for this
    // directory never race for `daemon.lock`.
    // The drain gets its own bound (the coordinator's stop grace plus a
    // margin), and the replacement then gets the intent's full window: a
    // loaded host can spend most of one shared budget on the drain alone.
    let deadline = if evicting {
        stop_if_healthy(paths, Instant::now() + EVICT_BOUND);
        Instant::now() + intent.bound()
    } else {
        deadline
    };
    // Stale metadata cannot prove process ownership after PID reuse. Only
    // the authenticated control connection above authorizes shutdown.

    let canonical = identity::canonical_data_dir(paths)?;
    let kind = supervisor::detect()?;
    if kind == supervisor::Kind::External {
        return wait_or_fail(paths, intent, kind, Vec::new(), deadline, "none");
    }
    let (chosen, notes) = start_backend(paths, &canonical, kind)?;
    let action = if evicting {
        "replaced"
    } else {
        action_of(intent)
    };
    wait_or_fail(paths, intent, chosen, notes, deadline, action)
}

/// Try `kind`; on any failure fall back to [`supervisor::Kind::Process`]
/// with a note (D2), unless `kind` already is `Process`. The coordinator
/// that answered before this call, if any, is already gone
/// ([`stop_if_healthy`]), so writing the unit never races a live process.
fn start_backend(
    paths: &Paths,
    canonical: &std::path::Path,
    kind: supervisor::Kind,
) -> Result<(supervisor::Kind, Vec<String>)> {
    if kind == supervisor::Kind::Unsupported {
        return Err(Error::Unsupported(
            "comemory sync daemon is not supported on this OS".into(),
        ));
    }
    if kind != supervisor::Kind::Process {
        let unit = supervisor::plan(canonical, kind)?;
        let me = identity::BinaryIdentity::current()?;
        supervisor::remove_legacy(kind);
        match supervisor::activate(kind, &unit, &me.path, canonical) {
            Ok(()) => return Ok((kind, Vec::new())),
            Err(e) => {
                spawn::spawn(paths)?;
                return Ok((
                    supervisor::Kind::Process,
                    vec![format!("{}: {e}; using process supervision", kind.as_str())],
                ));
            }
        }
    }
    spawn::spawn(paths)?;
    Ok((kind, Vec::new()))
}

const fn action_of(intent: Intent) -> &'static str {
    match intent {
        Intent::Restart => "restarted",
        Intent::Repair => "replaced",
        Intent::Preflight | Intent::Ensure => "started",
    }
}

fn wait_or_fail(
    paths: &Paths,
    intent: Intent,
    supervisor: supervisor::Kind,
    notes: Vec<String>,
    deadline: Instant,
    action: &'static str,
) -> Result<Ensured> {
    let current = identity::BinaryIdentity::current()?;
    // The last verified coordinator that was not this binary, if any.
    let mut other: Option<String>;
    // Why the last probe found no coordinator, for the error.
    let mut last_miss: Option<Probe> = None;
    loop {
        let probe = client::probe(paths, remaining(deadline, client::PROBE_BOUND));
        if let Probe::Healthy(readiness) = probe {
            if fits(settled(intent), &readiness, &current) {
                return Ok(Ensured {
                    ready: true,
                    action,
                    supervisor,
                    notes,
                    daemon: Some(*readiness),
                    error: None,
                });
            }
            other = Some(format!(
                "a coordinator (pid {}, {} at {}, file {}) answers instead of this binary (file {})",
                readiness.pid,
                readiness.version,
                readiness.binary.display(),
                readiness.binary_file.as_deref().unwrap_or("unknown"),
                current.file.as_deref().unwrap_or("unknown"),
            ));
        } else {
            other = None;
            last_miss = Some(probe);
        }
        if Instant::now() >= deadline {
            let error = other.unwrap_or_else(|| unanswered(paths, supervisor, last_miss.take()));
            return Ok(not_ready(supervisor, notes, error));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Why nothing answered by the deadline: the local-service fix, what the
/// last probe saw, and the process-supervised coordinator's own last log
/// line (a start-up failure such as a held `daemon.lock` shows up there).
fn unanswered(paths: &Paths, supervisor: supervisor::Kind, miss: Option<Probe>) -> String {
    let mut parts = vec![local_service_error(supervisor)];
    if let Some(Probe::NotRunning(why) | Probe::Stale(why)) = miss {
        parts.push(format!("last probe: {why}"));
    }
    if let Some(line) = spawn::last_log_line(paths) {
        parts.push(format!("coordinator log: {line}"));
    }
    parts.join("; ")
}

/// A not-ready result. The notes (a supervisor fallback, say) are folded
/// into the error too, so a caller that prints only the error — an
/// installer — still shows why the backend it expected was not used.
fn not_ready(supervisor: supervisor::Kind, notes: Vec<String>, error: String) -> Ensured {
    let error = if notes.is_empty() {
        error
    } else {
        format!("{error} [{}]", notes.join("; "))
    };
    Ensured {
        ready: false,
        action: "none",
        supervisor,
        notes,
        daemon: None,
        error: Some(error),
    }
}

/// The identity a finished repair must show: `repair` rewrote the unit for
/// this binary, so it settles on this binary exactly like `ensure`.
const fn settled(intent: Intent) -> Intent {
    match intent {
        Intent::Repair => Intent::Ensure,
        other => other,
    }
}

fn local_service_error(supervisor: supervisor::Kind) -> String {
    if supervisor == supervisor::Kind::External {
        return "sync daemon not ready (external) — restart your comemory sync daemon run under your supervisor so it runs this binary".into();
    }
    format!(
        "sync daemon not ready ({}) — run `comemory sync daemon run` in the foreground, or `comemory sync daemon repair`",
        supervisor.as_str()
    )
}

fn remaining(deadline: Instant, cap: Duration) -> Duration {
    deadline
        .checked_duration_since(Instant::now())
        .unwrap_or(Duration::ZERO)
        .min(cap)
}

#[cfg(test)]
#[path = "tests/ensure.rs"]
mod tests;
