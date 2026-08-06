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

use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::config::paths::Paths;
use crate::prelude::*;
use crate::store::{connection, repo_marker_roots};

pub mod assets;
pub mod envelope;
pub mod error;
pub mod fileio;
pub mod handlers;
pub mod repo_root;
pub mod router;
pub mod routes;
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
    /// `--allow-path <dir>` entries, already canonicalized by the CLI layer
    /// (a bad entry fails startup rather than being silently dropped — see
    /// `cli::serve::parse_allow_paths`). One source of the `contain_abs`
    /// allowed-roots set.
    pub allow_path: Vec<PathBuf>,
}

/// Shared, cheaply-cloneable handler state. The SQLite connection is wrapped
/// in a `Mutex` (rusqlite `Connection` is `Send` but not `Sync`); handlers
/// lock it only for the duration of a synchronous query and never hold the
/// guard across an `.await`.
#[derive(Clone)]
pub struct AppState {
    conn: Arc<Mutex<Connection>>,
    /// The data-dir layout this session was started with. `api::Ctx`
    /// (`src/api/mod.rs`) needs it for the commands whose middle touches
    /// the filesystem directly (`rebuild`'s atomic swap, `ast`, …).
    paths: Arc<Paths>,
    roots: Arc<RootOverrides>,
    token: Arc<str>,
    read_only: bool,
    repo: Option<String>,
    /// Swappable so a `tune`/`bandit --apply` job (later step) can replace
    /// the inner `Arc` after rewriting `config.toml`; every request clones
    /// out the current value via [`AppState::cfg`] rather than holding a
    /// long-lived reference.
    cfg: Arc<Mutex<Arc<Config>>>,
    embed_cmd: Option<Arc<str>>,
    /// Single write permit (§Concurrency) — held for the duration of every
    /// mutating job and, via `try_acquire`, every synchronous mutating
    /// request. Not yet wired to a handler (a later step); constructed here
    /// so it is available.
    write_permit: Arc<tokio::sync::Semaphore>,
    /// Canonicalized `--allow-path <dir>` entries — one source feeding the
    /// `contain_abs` allowed-roots set (§Security "Path containment").
    allow_path: Arc<Vec<PathBuf>>,
    /// The server process's own git work-tree root (canonicalized), when its
    /// cwd is inside one — the bootstrap case for a fresh install with no
    /// `repo_marker` rows yet.
    bootstrap_root: Option<Arc<PathBuf>>,
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

    /// The data-dir layout (`Paths`) this session was started with.
    pub fn paths(&self) -> &Paths {
        &self.paths
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

    /// The current layered config, cloned out of the swappable slot. A
    /// poisoned lock (a panic in another handler mid-swap) is recovered
    /// rather than propagated: the only mutation is a single `Arc` pointer
    /// swap, so a poisoned guard's inner value is never torn.
    pub(crate) fn cfg(&self) -> Arc<Config> {
        self.cfg
            .lock()
            .map(|g| Arc::clone(&g))
            .unwrap_or_else(|poisoned| Arc::clone(&poisoned.into_inner()))
    }

    /// The embed command for semantic web search, if configured.
    pub(crate) fn embed_cmd(&self) -> Option<&str> {
        self.embed_cmd.as_deref()
    }

    /// The single write permit (§Concurrency): mutating handlers/jobs
    /// `try_acquire`/`acquire` this before touching the store. Reserved here
    /// for the write-permit-wiring step (s2.2+); not yet called by a
    /// handler.
    pub fn write_permit(&self) -> &Arc<tokio::sync::Semaphore> {
        &self.write_permit
    }

    /// The full allowed-roots set for `security::contain_abs`: `--root`
    /// override paths (canonicalized here — the map's values are not
    /// guaranteed pre-canonical) ∪ every stored `repo_marker.root_path` ∪
    /// the server-cwd bootstrap root, if any ∪ `--allow-path` entries.
    /// Deduplicated. Best-effort against `conn`: a `repo_marker` read
    /// failure is logged and drops only that source, not the other three —
    /// it is an enrichment, not a hard requirement. Reserved here for the
    /// path-containment-wiring step (s2.2+); not yet called by a handler.
    pub fn allowed_roots(&self, conn: &Connection) -> Vec<PathBuf> {
        let mut set: HashSet<PathBuf> = HashSet::new();
        for p in self.roots.values() {
            if let Ok(canonical) = p.canonicalize() {
                set.insert(canonical);
            }
        }
        match repo_marker_roots::all_roots(conn) {
            Ok(roots) => set.extend(roots),
            Err(e) => tracing::warn!(error = %e, "allowed_roots: repo_marker read failed"),
        }
        if let Some(root) = &self.bootstrap_root {
            set.insert(root.as_ref().clone());
        }
        set.extend(self.allow_path.iter().cloned());
        set.into_iter().collect()
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
    // Hoisted here (rather than left to the CLI caller) so every route —
    // including read-only ones like `GET /doctor` — can rely on the data-dir
    // tree already existing, with no per-request side effect.
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;
    let token = security::generate_token()?;
    let state = AppState {
        conn: Arc::new(Mutex::new(conn)),
        paths: Arc::new(paths.clone()),
        roots: Arc::new(opts.roots),
        token: Arc::from(token.as_str()),
        read_only: opts.read_only,
        repo: opts.repo,
        cfg: Arc::new(Mutex::new(Arc::new(opts.cfg))),
        embed_cmd: opts.embed_cmd.map(Arc::from),
        write_permit: Arc::new(tokio::sync::Semaphore::new(1)),
        allow_path: Arc::new(opts.allow_path),
        bootstrap_root: discover_bootstrap_root().map(Arc::new),
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

/// Best-effort: when the server process's own cwd sits inside a git work
/// tree, that tree's canonicalized root — the bootstrap case for a fresh
/// install with no `repo_marker` rows yet. `None` on any failure (no cwd, no
/// repo, bare repo, uncanonicalizable workdir) — this is enrichment, not a
/// requirement, so a non-git cwd never blocks startup.
fn discover_bootstrap_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let repo = git2::Repository::discover(&cwd).ok()?;
    repo.workdir()?.canonicalize().ok()
}
