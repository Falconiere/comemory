//! The three `comemory auth` use cases, without their rendering.
//!
//! Each one is the sequence a login, a status probe or a logout performs
//! against the platform and the local credential file; the caller owns the
//! clap flags, the progress destination and every line of output.

use std::time::{Duration, Instant};

use crate::domains::sync::auto::PASS_LOCK;
use crate::domains::sync::cloud::{self, StatusReport};
use crate::domains::sync::daemon::client::{self, Probe};
use crate::domains::sync::daemon::control::Op;
use crate::domains::sync::drain::adopt::{self, Reach};
use crate::domains::sync::drain::{keying, network};
use crate::domains::sync::{auth_barrier, auth_file};
use crate::prelude::*;
use crate::store::connection;
use crate::store::sync_exchange::ExchangeKey;
use crate::utilities::file_lock::FileLock;

use super::AuthFile;
use crate::config::env;
use crate::config::sync::{daemon_disabled, parse_duration};
use crate::config::{Config, Paths};

/// How long a post-credential-change `reload`/probe may take. Bounded well
/// under the CLI's own patience — a slow coordinator must not turn a
/// finished login or logout into a hang.
const REACT_BOUND: Duration = Duration::from_secs(2);

/// What a successful login established, before anything is rendered.
pub struct Established {
    /// The org-scoped credential, already persisted to `auth.json`.
    pub credentials: AuthFile,
    /// Whether a verified coordinator answered after the `reload`.
    pub daemon_running: bool,
    /// That coordinator's instance id, when it answered.
    pub daemon_instance: Option<String>,
}

/// What a logout left behind.
pub struct LoggedOut {
    /// Whether a verified coordinator still answers (it is never stopped by
    /// logout, D11 — only its auth view changes).
    pub daemon_running: bool,
    /// What the wait for a pass already holding `sync.lock` found.
    pub in_flight: InFlight,
}

/// Whether a pass was running `sync.lock` when logout started, and whether
/// it finished before the bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InFlight {
    /// The lock was free immediately.
    None,
    /// It was held, but freed before the bound.
    Drained,
    /// It was still held when the bound ran out.
    TimedOut,
}

/// Device-login and persist the org-scoped credential.
///
/// `progress` receives the device-code prompt the operator has to act on.
/// The coordinator for this data directory already exists — preflight
/// ensures one before any `Required` command, including this one — so
/// login's only daemon-facing job is to tell it the credential changed
/// (`reload`, best-effort) and report whether it answered afterwards.
///
/// # Errors
/// Propagates API-URL resolution, the device flow, the credential write and
/// the stale-allowlist clear. The `reload` and its probe are best-effort and
/// never fail the login.
pub fn establish(
    (paths, cfg): (&Paths, &Config),
    api_url_override: Option<&str>,
    progress: &mut impl std::io::Write,
) -> Result<Established> {
    let api_url = cloud::resolve_api_url(api_url_override)?;
    let outcome = cloud::login(&api_url, progress)?;
    if let Some(previous) = outgoing(paths) {
        let key = |a: &AuthFile| ExchangeKey::new(&a.api_url, &a.workspace_id);
        if key(&previous) != key(&outcome.credentials) {
            stamp_outgoing((paths, cfg), &previous)?;
        }
    }
    outcome.credentials.save(paths)?;
    auth_file::clear_stale_allowlist(paths)?;
    let (daemon_running, daemon_instance) = reload_and_probe(paths);
    Ok(Established {
        credentials: outcome.credentials,
        daemon_running,
        daemon_instance,
    })
}

/// Probe whether this machine's credentials still authenticate.
///
/// The secret is the stored one, or `COMEMORY_API_KEY` when there is no file.
/// The API URL is the caller's override, else the file's, else the default.
/// `Ok(None)` means neither source offered a secret — the logged-out report,
/// which is not an error at this layer.
///
/// # Errors
/// Propagates a broken `auth.json`, an unresolvable API URL and the probe
/// itself.
pub fn status(paths: &Paths, api_url_override: Option<&str>) -> Result<Option<StatusReport>> {
    // After a logout an inherited `COMEMORY_API_KEY` must not read as a login.
    if auth_barrier::active(paths) {
        return Ok(None);
    }
    let file = AuthFile::load(paths)?;
    let secret = match &file {
        Some(creds) => Some(creds.effective_secret()),
        None => env::api_key_override(),
    };
    let Some(secret) = secret else {
        return Ok(None);
    };
    let api_url = if let Some(raw) = api_url_override {
        cloud::resolve_api_url(Some(raw))?
    } else if let Some(creds) = &file {
        creds.api_url.clone()
    } else {
        cloud::resolve_api_url(None)?
    };
    let mut report = cloud::org_status(&api_url, &secret, file.as_ref())?;
    if report.key_prefix.is_none() {
        report.key_prefix = file.as_ref().map(|c| c.key_prefix.clone());
    }
    Ok(Some(report))
}

/// Drop this machine's credentials. The coordinator is never stopped (D11):
/// the barrier goes up first, so every credential read answers logged out
/// and a pass mid-drain stops at its next batch boundary; then this waits
/// for `sync.lock` to free before the credential file goes.
///
/// There is no remote revoke: the key stays valid until the organization
/// rotates it.
///
/// # Errors
/// The barrier cannot be written, or (as [`forget`]) a store failure leaves
/// the credential in place.
pub fn logout(paths: &Paths, cfg: &Config) -> Result<LoggedOut> {
    // Read who is leaving before the barrier goes up: `AuthFile::load` (and
    // so `outgoing`) reads a standing barrier as logged out, which would
    // otherwise make the barrier-first ordering below skip the stamp.
    let leaving = outgoing(paths);
    auth_barrier::raise(paths)?;
    let in_flight = wait_for_pass_lock(paths, wait_bound(cfg)?);
    forget_with(paths, cfg, leaving)?;
    let (daemon_running, _instance) = reload_and_probe(paths);
    Ok(LoggedOut {
        daemon_running,
        in_flight,
    })
}

/// Best-effort `reload` (the coordinator re-reads `auth.json`/config) then a
/// probe for its resulting readiness. A no-op under the harness switch,
/// where preflight never ensured anything to reload.
fn reload_and_probe(paths: &Paths) -> (bool, Option<String>) {
    if daemon_disabled() {
        return (false, None);
    }
    let _ = client::call::<serde_json::Value>(paths, &Op::Reload {}, REACT_BOUND);
    match client::probe(paths, REACT_BOUND) {
        Probe::Healthy(readiness) => (true, Some(readiness.instance.clone())),
        Probe::NotRunning(_) | Probe::Stale(_) => (false, None),
    }
}

/// `request_timeout` plus a margin, matching the exchange's own request
/// budget: a pass blocked on the network for up to `request_timeout` is
/// still "in flight", not stuck.
fn wait_bound(cfg: &Config) -> Result<Duration> {
    Ok(parse_duration(&cfg.sync.request_timeout)? + Duration::from_secs(5))
}

/// Wait up to `bound` for `sync.lock` to be free. A lock that cannot even be
/// opened (permissions, disk full) counts as free — best-effort, and there
/// is nothing else to wait on.
fn wait_for_pass_lock(paths: &Paths, bound: Duration) -> InFlight {
    let path = paths.data_dir().join(PASS_LOCK);
    match FileLock::try_acquire(&path, "sync") {
        Ok(Some(_lock)) => InFlight::None,
        Ok(None) => {
            let deadline = Instant::now() + bound;
            while Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
                if matches!(FileLock::try_acquire(&path, "sync"), Ok(Some(_))) {
                    return InFlight::Drained;
                }
            }
            InFlight::TimedOut
        }
        Err(e) => {
            tracing::warn!(error = %e, "logout: sync.lock probe failed; proceeding");
            InFlight::None
        }
    }
}

/// The credential half of a logout: stamp every still-unstamped pending row
/// with the key being left — they were made under it, and no later key may
/// send them — then remove `auth.json`.
///
/// # Errors
/// Propagates the store — queueing or stamping what the outgoing key owes —
/// before the credential is touched, so a failure leaves it in place and no
/// run made under it can reach another workspace; retrying once the store is
/// writable finishes the logout. Then propagates the credential removal.
pub fn forget(paths: &Paths, cfg: &Config) -> Result<()> {
    forget_with(paths, cfg, outgoing(paths))
}

/// [`forget`]'s body, taking the outgoing credential rather than re-reading
/// it — [`logout`] must read it before raising the barrier.
fn forget_with(paths: &Paths, cfg: &Config, leaving: Option<AuthFile>) -> Result<()> {
    if let Some(leaving) = leaving {
        stamp_outgoing((paths, cfg), &leaving)?;
    }
    AuthFile::clear(paths)
}

/// The credential being replaced or removed. One that cannot be read (a
/// pre-organization v1 file, a corrupt one) names no key to stamp with — it
/// must still be replaceable and removable, so that is a warning, not a
/// failure: changes owed under it stay unstamped.
fn outgoing(paths: &Paths) -> Option<AuthFile> {
    match AuthFile::load(paths) {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!(error = %e, "the outgoing credential is unreadable; changes owed under it are not stamped with its workspace");
            None
        }
    }
}

/// Stamp every unstamped pending row with the key `leaving` names. A machine
/// with no store yet has nothing to stamp, and gets no store from this.
///
/// Runs recorded under that key are captured and queued first: a run reaches
/// the outbox only when a pass adopts it, and the next key's first pass would
/// otherwise adopt it and send it to the workspace being entered.
fn stamp_outgoing((paths, cfg): (&Paths, &Config), leaving: &AuthFile) -> Result<()> {
    if !paths.db_path().exists() {
        return Ok(());
    }
    let mut conn = connection::open(paths.db_path())?;
    let key = ExchangeKey::new(&leaving.api_url, &leaving.workspace_id);
    let at = network::now()?;
    adopt::events((paths, cfg), &mut conn, &at, Reach::All)?;
    keying::stamp_outgoing(&conn, &key, &at)?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/login.rs"]
mod tests;
