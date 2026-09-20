//! Shared, cheaply-cloneable state for one `comemory mcp` session.
//!
//! `serve::AppState`'s shape without anything a socket needs: no token, no
//! port or job registry. A session mutex serializes tool calls inside the
//! blocking task. Each call opens and drops its own connection so idle
//! agents cannot retain an obsolete database after `comemory rebuild`.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::config::paths::Paths;
use crate::config::{Config, env};
use crate::mcp::{McpOptions, scope};
use crate::prelude::*;
use crate::store::{Connection, connection};
use crate::utilities::activity::Origin;

/// Everything a tool body needs: the session gate, the data-dir layout,
/// the layered config, the session's default scope and its refusal flag.
#[derive(Clone)]
pub struct McpState {
    gate: Arc<Mutex<()>>,
    paths: Arc<Paths>,
    cfg: Arc<Config>,
    repo: Option<String>,
    read_only: bool,
    /// `"<clientInfo.name>/<version>"`, recorded once at `initialize` and
    /// reported as the `actor` of every row this session's calls write. A
    /// `OnceLock` rather than a `Mutex`: a session initializes exactly once,
    /// and a host that sends no `clientInfo` leaves it empty forever.
    client: Arc<OnceLock<String>>,
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
        drop(connection::open(paths.db_path())?);
        Ok(Self {
            gate: Arc::new(Mutex::new(())),
            paths: Arc::new(paths.clone()),
            cfg: Arc::new(opts.cfg),
            repo: scope::default_repo(opts.repo, cwd),
            read_only: opts.read_only,
            client: Arc::new(OnceLock::new()),
        })
    }

    /// Record the host that opened this session, once — `ComemoryServer`'s
    /// `initialize` override calls it with the `clientInfo` the host sent.
    /// A second call is ignored: a session has one client.
    pub fn note_client(&self, label: String) {
        let _ = self.client.set(label);
    }

    /// This session's activity origin: `source = "mcp"`, the host's label as
    /// `actor` when it sent one, and recording off entirely on a
    /// `--read-only` session (which writes nothing to the store, telemetry
    /// included).
    pub fn origin(&self) -> Origin {
        let origin = Origin::mcp(&self.cfg, self.client.get().map(String::as_str));
        if self.read_only {
            origin.read_only()
        } else {
            origin
        }
    }

    /// Serialize this session's tool calls, mapping poisoning to an error.
    pub fn lock(&self) -> Result<MutexGuard<'_, ()>> {
        self.gate
            .lock()
            .map_err(|_| Error::Other("mcp: session lock poisoned".into()))
    }

    /// Open the current database for one call, including after a rebuild.
    /// Callers hold [`Self::lock`] until this connection has been dropped.
    pub fn conn(&self) -> Result<Connection> {
        connection::open(self.paths.db_path())
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
