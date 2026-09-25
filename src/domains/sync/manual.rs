//! What one `comemory sync` run does, without its reports.
//!
//! A run needs a credential and an open store before it can do anything.
//! Then, under the sync pass lock: a secret override is recorded, stale
//! hooked repos are refreshed (so a code capture sees their latest HEAD), and
//! the key is drained ([`crate::domains::sync::drain`]) until a pass ends
//! without `more` — no per-run cap. A legacy key on a managed origin then
//! pushes the code index as before. The caller owns the flags, the
//! blocking-I/O isolation and every emitted line.

use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::config::sync::apply_embed_model;
use crate::config::{Config, Paths};
use crate::domains::code::hooked_refresh::{self, RefreshStats};
use crate::domains::sync::auto::hold_pass_lock;
use crate::domains::sync::code::{self, CodePushStats};
use crate::domains::sync::drain::{
    self,
    report::Report,
    session::{Legs, Mode},
};
use crate::domains::sync::pull::PullStats;
use crate::domains::sync::push::PushStats;
use crate::prelude::*;
use crate::store::{Connection, sync_binding};

use super::AuthFile;

/// The credential and open store a manual run works through.
pub struct Session {
    /// The org-scoped credential this run is authenticated by.
    pub auth: AuthFile,
    /// The store connection, already carrying the configured embed model.
    pub conn: Connection,
}

/// What one run did. A field is `None` when that leg was not part of the
/// requested action, which is how the report distinguishes "did not run" from
/// "ran and moved nothing". On a `replica-v1` key the `exchange` leg replaces
/// `pull`, `push` and `code`.
#[derive(Debug, Default)]
pub struct RunStats {
    /// The legacy pull leg, present for `run` and `pull` on a legacy key.
    pub pull: Option<PullStats>,
    /// The legacy memory push leg, present for `run` and `push` on a legacy key.
    pub push: Option<PushStats>,
    /// The code-index push, present for `run` and `push` on a legacy key of a
    /// managed origin.
    pub code: Option<CodePushStats>,
    /// The hooked-repo refresh that precedes the push, present for `run` and
    /// `push` (and for every `--action auto` pass).
    pub refresh: Option<RefreshStats>,
    /// Every pass of the drain, folded together.
    pub exchange: Option<Report>,
    /// The key's recorded error, when the drain ended on the network — the
    /// report is still written, then the run fails with it.
    pub error: Option<String>,
}

/// Load the credential, open the store, and apply the configured embed model,
/// in that order — so a machine that is not logged in is refused before a
/// database is opened, let alone migrated.
///
/// # Errors
/// [`Error::Usage`] when there is no credential; otherwise propagates the
/// credential read, the connection and the embed-model check.
pub fn open_session(paths: &Paths, cfg: &Config) -> Result<Session> {
    let auth = AuthFile::load(paths)?
        .ok_or_else(|| Error::Usage("not logged in — run `comemory auth login`".into()))?;
    paths.ensure_dirs()?;
    let conn = crate::store::connection::open(paths.db_path())?;
    apply_embed_model(&conn, &cfg.embed)?;
    Ok(Session { auth, conn })
}

/// One manual run of `legs` under the sync pass lock.
///
/// # Errors
/// Propagates the lock, the refresh, the drain and the code push; network
/// failures are reported in the `exchange` leg instead.
pub fn run(
    paths: &Paths,
    cfg: &Config,
    session: &mut Session,
    allow_secret: Option<&str>,
    legs: Legs,
) -> Result<RunStats> {
    let _pass = hold_pass_lock(paths)?;
    let conn = &mut session.conn;
    if let Some(id) = allow_secret {
        let at = OffsetDateTime::now_utc()
            .format(&Iso8601::DEFAULT)
            .map_err(|e| Error::Other(format!("timestamp: {e}")))?;
        sync_binding::allow_secret(conn, id, &session.auth.workspace_id, "cli_override", &at)?;
    }
    let mut stats = RunStats::default();
    if legs.pushes() {
        let mut refresh = RefreshStats::default();
        hooked_refresh::refresh_stale(paths, cfg, conn, None, &mut refresh)?;
        stats.refresh = Some(refresh);
    }
    let drained = drain::drain(paths, cfg, conn, &session.auth, (Mode::Manual, legs))?;
    stats.exchange = Some(drained.exchange);
    stats.error = drained.error;
    if let Some(legacy) = drained.legacy {
        stats.pull = legacy.pull;
        stats.push = legacy.push;
        if legs.pushes() && drained.managed && stats.error.is_none() {
            stats.code = Some(code::run_code_push(cfg, conn, &session.auth)?);
        }
    }
    Ok(stats)
}

#[cfg(test)]
#[path = "tests/manual.rs"]
mod tests;
