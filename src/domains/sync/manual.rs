//! What one `comemory sync` run does, without its reports.
//!
//! A run needs a credential and an open store before it can do anything, and
//! three of the five actions are compositions rather than a single call: a
//! push is followed by the code-index push, and a full run pulls first. Those
//! sequences live here; the caller owns the flags, the blocking-I/O isolation
//! and every emitted line.

use crate::config::sync::apply_embed_model;
use crate::config::{Config, Paths};
use crate::domains::sync::code::{self, CodePushStats};
use crate::domains::sync::pull::{self, PullStats};
use crate::domains::sync::push::{self, PushStats};
use crate::prelude::*;
use crate::store::Connection;

use super::AuthFile;

/// Entries one manual run may push or pull per direction.
pub const RUN_LIMIT: usize = 2000;

/// The credential and open store a manual run works through.
pub struct Session {
    /// The org-scoped credential this run is authenticated by.
    pub auth: AuthFile,
    /// The store connection, already carrying the configured embed model.
    pub conn: Connection,
}

/// What one run did. A field is `None` when that leg was not part of the
/// requested action, which is how the report distinguishes "did not run" from
/// "ran and moved nothing".
#[derive(Default)]
pub struct RunStats {
    /// The pull leg, present for `run` and `pull`.
    pub pull: Option<PullStats>,
    /// The memory push leg, present for `run` and `push`.
    pub push: Option<PushStats>,
    /// The code-index push, present for `run` and `push`.
    pub code: Option<CodePushStats>,
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

/// Pull, then push, then push the code index — the default action.
///
/// # Errors
/// Propagates the first leg that fails; a later leg is then not attempted.
pub fn run_all(
    paths: &Paths,
    cfg: &Config,
    session: &mut Session,
    allow_secret: Option<&str>,
    limit: usize,
) -> Result<RunStats> {
    let pulled = pull::run_pull(paths, cfg, &mut session.conn, &session.auth, limit)?;
    let mut stats = push_then_code(paths, cfg, session, allow_secret, limit)?;
    stats.pull = Some(pulled);
    Ok(stats)
}

/// Push local changes, then push the code index.
///
/// # Errors
/// Propagates the memory push; the code push then runs only if it succeeded.
pub fn push_only(
    paths: &Paths,
    cfg: &Config,
    session: &mut Session,
    allow_secret: Option<&str>,
    limit: usize,
) -> Result<RunStats> {
    push_then_code(paths, cfg, session, allow_secret, limit)
}

/// Pull remote changes only.
///
/// # Errors
/// Propagates the cursored pull.
pub fn pull_only(
    paths: &Paths,
    cfg: &Config,
    session: &mut Session,
    limit: usize,
) -> Result<RunStats> {
    Ok(RunStats {
        pull: Some(pull::run_pull(
            paths,
            cfg,
            &mut session.conn,
            &session.auth,
            limit,
        )?),
        ..RunStats::default()
    })
}

/// The push half both `run` and `push` share: memories first, then the code
/// index, so the platform never sees code rows for memories it has not got.
fn push_then_code(
    paths: &Paths,
    cfg: &Config,
    session: &mut Session,
    allow_secret: Option<&str>,
    limit: usize,
) -> Result<RunStats> {
    let pushed = push::run_push(
        paths,
        cfg,
        &mut session.conn,
        &session.auth,
        allow_secret,
        limit,
    )?;
    let code = code::run_code_push(cfg, &mut session.conn, &session.auth)?;
    Ok(RunStats {
        pull: None,
        push: Some(pushed),
        code: Some(code),
    })
}

#[cfg(test)]
#[path = "tests/manual.rs"]
mod tests;
