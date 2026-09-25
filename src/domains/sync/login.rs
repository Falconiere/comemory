//! The three `comemory auth` use cases, without their rendering.
//!
//! Each one is the sequence a login, a status probe or a logout performs
//! against the platform and the local credential file; the caller owns the
//! clap flags, the progress destination and every line of output.

use crate::domains::sync::cloud::{self, StatusReport};
use crate::domains::sync::drain::{keying, network};
use crate::domains::sync::{auth_file, daemon};
use crate::prelude::*;
use crate::store::connection;
use crate::store::sync_exchange::ExchangeKey;

use super::AuthFile;
use crate::config::Paths;
use crate::config::env;

/// What a successful login established, before anything is rendered.
pub struct Established {
    /// The org-scoped credential, already persisted to `auth.json`.
    pub credentials: AuthFile,
    /// `true` when no daemon install was requested, so none was attempted.
    pub daemon_skipped: bool,
    /// Whether the daemon is running, when an install was attempted and its
    /// status probe answered.
    pub daemon_running: Option<bool>,
}

/// Device-login, persist the org-scoped credential, and — only when asked —
/// install and start the sync daemon.
///
/// `progress` receives the device-code prompt the operator has to act on.
/// The daemon stays opt-in: without `install_daemon` nothing is written to
/// the supervisor, because a save pushes inline and `comemory watch` covers
/// the pull direction.
///
/// # Errors
/// Propagates API-URL resolution, the device flow, the credential write and
/// the stale-allowlist clear. A daemon install is best-effort and never fails
/// the login.
pub fn establish(
    paths: &Paths,
    api_url_override: Option<&str>,
    install_daemon: bool,
    progress: &mut impl std::io::Write,
) -> Result<Established> {
    let api_url = cloud::resolve_api_url(api_url_override)?;
    let outcome = cloud::login(&api_url, progress)?;
    if let Some(previous) = outgoing(paths) {
        let key = |a: &AuthFile| ExchangeKey::new(&a.api_url, &a.workspace_id);
        if key(&previous) != key(&outcome.credentials) {
            stamp_outgoing(paths, &previous)?;
        }
    }
    outcome.credentials.save(paths)?;
    auth_file::clear_stale_allowlist(paths)?;
    if !install_daemon {
        return Ok(Established {
            credentials: outcome.credentials,
            daemon_skipped: true,
            daemon_running: None,
        });
    }
    daemon::install_and_start_best_effort(paths);
    Ok(Established {
        daemon_running: daemon::status().ok().map(|s| s.running),
        credentials: outcome.credentials,
        daemon_skipped: false,
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

/// Drop this machine's credentials and stop the daemon. Returns whether the
/// daemon is stopped afterwards; an unavailable status probe counts as
/// stopped, since there is then nothing to report as still running.
///
/// There is no remote revoke: the key stays valid until the organization
/// rotates it.
///
/// # Errors
/// Propagates a failure to remove `auth.json`.
pub fn logout(paths: &Paths) -> Result<bool> {
    // Stop while credentials still exist so a failing stop does not leave the
    // daemon racing against a deleted auth.json mid-clear.
    daemon::stop_best_effort();
    let daemon_stopped = daemon::status().map_or(true, |s| !s.running);
    forget(paths)?;
    Ok(daemon_stopped)
}

/// The credential half of a logout: stamp every still-unstamped pending row
/// with the key being left — they were made under it, and no later key may
/// send them — then remove `auth.json`.
///
/// # Errors
/// Propagates the store and the credential removal.
pub fn forget(paths: &Paths) -> Result<()> {
    if let Some(leaving) = outgoing(paths) {
        stamp_outgoing(paths, &leaving)?;
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
fn stamp_outgoing(paths: &Paths, leaving: &AuthFile) -> Result<()> {
    if !paths.db_path().exists() {
        return Ok(());
    }
    let conn = connection::open(paths.db_path())?;
    let key = ExchangeKey::new(&leaving.api_url, &leaving.workspace_id);
    keying::stamp_outgoing(&conn, &key, &network::now()?)?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/login.rs"]
mod tests;
