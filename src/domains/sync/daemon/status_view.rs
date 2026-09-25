//! `comemory sync daemon status`: a live probe report that starts nothing.

use serde::Serialize;

use crate::config::Paths;
use crate::domains::sync::daemon::client::{self, Probe};
use crate::domains::sync::daemon::readiness::Readiness;
use crate::domains::sync::daemon::supervisor;
use crate::prelude::*;

/// The coordinator's live state, from this directory's own point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// A verified coordinator answers.
    Running,
    /// Nothing answers.
    NotRunning,
    /// Something answers but does not verify (foreign, another directory,
    /// another protocol, or timed out).
    Stale,
    /// `COMEMORY_SYNC_DAEMON=0`.
    Disabled,
    /// This OS has no supported backend.
    Unsupported,
}

/// The full `status` report.
#[derive(Debug, Clone, Serialize)]
pub struct StatusView {
    /// One of the [`State`] values.
    pub state: State,
    /// One human-readable line.
    pub detail: String,
    /// Whether a running coordinator's version matches this binary.
    pub version_matches: Option<bool>,
    /// The detected/attempted backend.
    pub supervisor: &'static str,
    /// Full readiness, when [`State::Running`].
    pub daemon: Option<Readiness>,
}

/// Probe the coordinator for `paths`.
///
/// # Errors
/// Propagates only `supervisor::detect`'s config error (an unrecognized
/// `COMEMORY_DAEMON_SUPERVISOR` override).
pub fn view(paths: &Paths) -> Result<StatusView> {
    if crate::config::sync::daemon_disabled() {
        return Ok(StatusView {
            state: State::Disabled,
            detail: "disabled (COMEMORY_SYNC_DAEMON=0)".into(),
            version_matches: None,
            supervisor: supervisor::Kind::External.as_str(),
            daemon: None,
        });
    }
    let kind = supervisor::detect()?;
    if kind == supervisor::Kind::Unsupported {
        return Ok(StatusView {
            state: State::Unsupported,
            detail: "the sync daemon is not supported on this OS (macOS and Linux only)".into(),
            version_matches: None,
            supervisor: kind.as_str(),
            daemon: None,
        });
    }
    match client::probe(paths, client::PROBE_BOUND) {
        Probe::Healthy(readiness) => {
            let matches = readiness.version == env!("CARGO_PKG_VERSION");
            Ok(StatusView {
                state: State::Running,
                detail: format!("running (pid {}, v{})", readiness.pid, readiness.version),
                version_matches: Some(matches),
                supervisor: kind.as_str(),
                daemon: Some(*readiness),
            })
        }
        Probe::NotRunning(why) => Ok(StatusView {
            state: State::NotRunning,
            detail: format!("not running: {why}"),
            version_matches: None,
            supervisor: kind.as_str(),
            daemon: None,
        }),
        Probe::Stale(why) => Ok(StatusView {
            state: State::Stale,
            detail: format!("stale: {why}"),
            version_matches: None,
            supervisor: kind.as_str(),
            daemon: None,
        }),
    }
}

#[cfg(test)]
#[path = "tests/status_view.rs"]
mod tests;
