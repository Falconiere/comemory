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
    let same_binary = readiness.version == current.version && readiness.binary == current.path;
    let acceptable =
        same_binary || (matches!(intent, Intent::Preflight) && readiness.binary.exists());
    acceptable.then(|| Ensured {
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
    }
}

/// Serialize on [`ENSURE_LOCK`], re-probe (a racer may have just fixed it),
/// then write/start the backend `[`supervisor::detect`]` chooses.
fn repair(paths: &Paths, intent: Intent, deadline: Instant) -> Result<Ensured> {
    let lock_path = paths.data_dir().join(ENSURE_LOCK);
    let lock = loop {
        if let Some(lock) = FileLock::try_acquire(&lock_path, "daemon-ensure")? {
            break Some(lock);
        }
        if let Probe::Healthy(readiness) = client::probe(paths, Duration::from_millis(200)) {
            return Ok(Ensured {
                ready: true,
                action: "none",
                supervisor: supervisor::detect().unwrap_or(supervisor::Kind::External),
                notes: vec!["a concurrent ensure already repaired the coordinator".into()],
                daemon: Some(*readiness),
                error: None,
            });
        }
        if Instant::now() >= deadline {
            break None;
        }
    };
    let Some(_lock) = lock else {
        return Ok(timed_out(paths, deadline));
    };

    // The lock may have waited; a healthy coordinator can already exist.
    let probe = client::probe(paths, Duration::from_millis(200));
    let evicting = matches!(probe, Probe::Healthy(_));
    if let Some(accepted) = accept(intent, probe) {
        return Ok(accepted);
    }
    // Whatever answered has the wrong identity (or nothing does): evict it
    // gracefully before starting a fresh one, so two coordinators for this
    // directory never race for `daemon.lock`.
    if evicting {
        stop_if_healthy(paths, deadline);
    }

    let canonical = identity::canonical_data_dir(paths)?;
    let kind = supervisor::detect()?;
    if kind == supervisor::Kind::External {
        return wait_or_fail(paths, kind, Vec::new(), deadline, "none");
    }
    let (chosen, notes) = start_backend(paths, &canonical, kind)?;
    let action = if evicting {
        "replaced"
    } else {
        action_of(intent)
    };
    wait_or_fail(paths, chosen, notes, deadline, action)
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
    supervisor: supervisor::Kind,
    notes: Vec<String>,
    deadline: Instant,
    action: &'static str,
) -> Result<Ensured> {
    loop {
        if let Probe::Healthy(readiness) =
            client::probe(paths, remaining(deadline, client::PROBE_BOUND))
        {
            return Ok(Ensured {
                ready: true,
                action,
                supervisor,
                notes,
                daemon: Some(*readiness),
                error: None,
            });
        }
        if Instant::now() >= deadline {
            return Ok(Ensured {
                ready: false,
                action: "none",
                supervisor,
                notes,
                daemon: None,
                error: Some(local_service_error(supervisor)),
            });
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn timed_out(paths: &Paths, deadline: Instant) -> Ensured {
    let supervisor = supervisor::detect().unwrap_or(supervisor::Kind::Process);
    match client::probe(paths, remaining(deadline, Duration::from_millis(200))) {
        Probe::Healthy(readiness) => Ensured {
            ready: true,
            action: "none",
            supervisor,
            notes: Vec::new(),
            daemon: Some(*readiness),
            error: None,
        },
        _ => Ensured {
            ready: false,
            action: "none",
            supervisor,
            notes: Vec::new(),
            daemon: None,
            error: Some(local_service_error(supervisor)),
        },
    }
}

fn local_service_error(supervisor: supervisor::Kind) -> String {
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
