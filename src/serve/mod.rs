//! `comemory serve` — a loopback-only HTTP server backing the interactive web
//! viewer + in-browser code editor.
//!
//! The server (axum, bound to `127.0.0.1`) hands out an embedded React/Vite
//! single-page app and a small JSON/file API over `comemory.db` and the
//! indexed source tree. Every file read and write is gated by a per-session
//! token, a loopback Host-header guard, and a canonicalize-and-contain path
//! check (see [`security`]). The graph payload is built by the same
//! `cli::graph::build_code_graph` the static `--format html` export uses, so
//! the two renderers never drift.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::config::paths::Paths;
use crate::prelude::*;
use crate::store::connection;

pub mod assets;
pub mod error;
pub mod fileio;
pub mod handlers;
pub mod repo_root;
pub mod router;
pub mod search;
pub mod security;

pub use repo_root::RootOverrides;

/// Caller-supplied configuration for one `comemory serve` session.
pub struct ServeOptions {
    /// Restrict the graph to one repo label (as `graph --repo` does).
    pub repo: Option<String>,
    /// TCP port to bind on loopback; `0` selects an ephemeral port.
    pub port: u16,
    /// Refuse all writes (`PUT /api/file` → 405) when true.
    pub read_only: bool,
    /// `--root <repo>=<path>` overrides for repo-root resolution.
    pub roots: RootOverrides,
    /// Best-effort open the printed URL in the user's browser.
    pub open: bool,
    /// Layered config, threaded to the search helper's ranking knobs.
    pub cfg: Config,
    /// Embed command for semantic web search (`--embed-cmd` /
    /// `COMEMORY_EMBED_CMD`). Unset → `/api/search` stays lexical.
    pub embed_cmd: Option<String>,
}

/// Shared, cheaply-cloneable handler state. The SQLite connection is wrapped
/// in a `Mutex` (rusqlite `Connection` is `Send` but not `Sync`); handlers
/// lock it only for the duration of a synchronous query and never hold the
/// guard across an `.await`.
#[derive(Clone)]
pub struct AppState {
    conn: Arc<Mutex<Connection>>,
    roots: Arc<RootOverrides>,
    token: Arc<str>,
    read_only: bool,
    repo: Option<String>,
    cfg: Arc<Config>,
    embed_cmd: Option<Arc<str>>,
}

impl AppState {
    /// Lock the shared connection. Maps lock poisoning (a panic in another
    /// handler while holding the guard) to an internal error rather than
    /// propagating the panic.
    pub(crate) fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| Error::Other("serve: database lock poisoned".into()))
    }

    /// The `--root` overrides for this session.
    pub(crate) fn roots(&self) -> &RootOverrides {
        &self.roots
    }

    /// The per-session bearer token.
    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    /// Whether writes are disabled for this session.
    pub(crate) fn read_only(&self) -> bool {
        self.read_only
    }

    /// The repo-label graph filter, if any.
    pub(crate) fn repo(&self) -> Option<&str> {
        self.repo.as_deref()
    }

    /// The layered config for this session (ranking knobs, page size).
    pub(crate) fn cfg(&self) -> &Config {
        &self.cfg
    }

    /// The embed command for semantic web search, if configured.
    pub(crate) fn embed_cmd(&self) -> Option<&str> {
        self.embed_cmd.as_deref()
    }
}

/// What the startup banner reports (also the `--json` payload).
#[derive(Serialize)]
struct ServeInfo<'a> {
    url: &'a str,
    port: u16,
    token: &'a str,
    read_only: bool,
}

/// Open `comemory.db`, build the handler state + router, bind a loopback
/// listener, print the access URL (carrying the token), and serve until the
/// process is interrupted.
pub async fn serve(paths: &Paths, opts: ServeOptions, json: bool) -> Result<()> {
    let conn = connection::open(paths.db_path())?;
    let token = security::generate_token()?;
    let state = AppState {
        conn: Arc::new(Mutex::new(conn)),
        roots: Arc::new(opts.roots),
        token: Arc::from(token.as_str()),
        read_only: opts.read_only,
        repo: opts.repo,
        cfg: Arc::new(opts.cfg),
        embed_cmd: opts.embed_cmd.map(Arc::from),
    };

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, opts.port))
        .await
        .map_err(Error::Io)?;
    let port = listener.local_addr().map_err(Error::Io)?.port();
    let url = format!("http://127.0.0.1:{port}/?token={token}");
    emit_banner(&url, port, &token, opts.read_only, json)?;
    if opts.open {
        open_browser(&url);
    }

    let app = router::build_router(state);
    axum::serve(listener, app).await.map_err(Error::Io)?;
    Ok(())
}

/// Print the access URL to stdout. Uses the `output` module (not `tracing`,
/// which is silent without `RUST_LOG`) so the URL+token is always visible and
/// machine-readable under `--json`.
fn emit_banner(url: &str, port: u16, token: &str, read_only: bool, json: bool) -> Result<()> {
    if json {
        return crate::output::json::write(&ServeInfo {
            url,
            port,
            token,
            read_only,
        });
    }
    let mode = if read_only { " (read-only)" } else { "" };
    crate::output::tty::header(&format!("comemory serve{mode} → {url}"))
}

/// Best-effort: hand the URL to the platform browser opener. Failures are
/// swallowed — the URL is already printed, so the user can open it manually.
fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(url).spawn();
}
