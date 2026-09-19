//! Shared, cheaply-cloneable state for one `comemory mcp` session.
//!
//! `serve::AppState`'s shape without anything a socket needs: no token, no
//! port, no job registry, no write permit. The connection is behind a
//! `Mutex` (it is `Send` but not `Sync`); every holder locks it inside a
//! blocking task only — see [`crate::mcp::exec`].

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::config::paths::Paths;
use crate::config::{Config, env};
use crate::mcp::{McpOptions, scope};
use crate::prelude::*;
use crate::store::{Connection, connection};

/// Everything a tool body needs: the shared connection, the data-dir layout,
/// the layered config, the session's default scope and its refusal flag.
#[derive(Clone)]
pub struct McpState {
    conn: Arc<Mutex<Connection>>,
    paths: Arc<Paths>,
    cfg: Arc<Config>,
    repo: Option<String>,
    read_only: bool,
}

impl McpState {
    /// Ensure the data-dir tree exists, open `comemory.db` under `paths`, and
    /// resolve the session's default repo scope from `opts.repo` or `cwd`.
    ///
    /// A store that cannot be opened propagates: the process exits and the
    /// host reports a failed server rather than answering every tool with an
    /// error (§ Failure modes).
    pub fn new(paths: &Paths, opts: McpOptions, cwd: &Path) -> Result<Self> {
        paths.ensure_dirs()?;
        let conn = connection::open(paths.db_path())?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            paths: Arc::new(paths.clone()),
            cfg: Arc::new(opts.cfg),
            repo: scope::default_repo(opts.repo, cwd),
            read_only: opts.read_only,
        })
    }

    /// Lock the shared connection, mapping lock poisoning (a panic in another
    /// tool while holding the guard) to an internal error instead of
    /// propagating the panic across the protocol loop.
    pub fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| Error::Other("mcp: database lock poisoned".into()))
    }

    /// The data-dir layout this session was started with.
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// The layered config every core's ranking knobs read.
    pub fn cfg(&self) -> &Config {
        &self.cfg
    }

    /// The session-wide default repo scope, if one was given or derived.
    pub fn repo(&self) -> Option<&str> {
        self.repo.as_deref()
    }

    /// Whether every mutating tool is refused for this session.
    pub fn read_only(&self) -> bool {
        self.read_only
    }

    /// Whether a read should log a `retrieval_log` row. A `--read-only`
    /// session writes nothing at all, including telemetry (AC-4); otherwise
    /// the `COMEMORY_DISABLE_ACCESS_TRACKING` hook decides, through the one
    /// definition `cli` and `serve` also read.
    pub fn track(&self) -> Result<bool> {
        if self.read_only {
            return Ok(false);
        }
        env::access_tracking_enabled()
    }
}

#[cfg(test)]
#[path = "tests/state.rs"]
mod tests;
