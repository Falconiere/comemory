//! `comemory serve` — a loopback-only HTTP server exposing the versioned
//! `/api/v1` REST surface (`routes`) over `comemory.db` and the indexed
//! source tree, for the console and any local agent or script.
//!
//! The server (axum, bound to `127.0.0.1`) runs every request through a
//! per-session token check and a loopback Host-header guard, and every
//! mutating route that takes a filesystem path through a
//! canonicalize-and-contain check (see [`security`]). Command logic lives in
//! `api::` — the same cores the CLI calls — so the two surfaces cannot
//! drift.

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

pub mod envelope;
pub mod jobs;
pub mod repo_root;
pub mod router;
pub mod routes;
pub mod scope;
pub mod security;

pub use repo_root::RootOverrides;

/// Caller-supplied configuration for one `comemory serve` session.
pub struct ServeOptions {
    /// Default repo scope for every read that accepts a `repo` filter —
    /// the last fallback behind an explicit `repo` parameter and the
    /// `X-Comemory-Repo` header (see `serve::scope::RepoScope`).
    pub repo: Option<String>,
    /// TCP port to bind on loopback; `0` selects an ephemeral port.
    pub port: u16,
    /// Refuse every `mutating` `/api/v1` route (`405 read_only`) when true.
    pub read_only: bool,
    /// `--root <repo>=<path>` overrides for repo-root resolution.
    pub roots: RootOverrides,
    /// Layered config, threaded to every route's ranking knobs.
    pub cfg: Config,
    /// Embed command (`--embed-cmd` / `COMEMORY_EMBED_CMD`) for the routes
    /// that vectorize on the server's behalf (`POST /doctor/reembed`).
    /// Unset → those routes answer `503 embedder_unavailable`.
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
    /// (`src/api.rs`) needs it for the commands whose middle touches
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
    /// request. Consumed by [`crate::serve::routes::guard_mutating`] (the
    /// synchronous `try_acquire` gate) and `crate::serve::jobs::spawn_job`
    /// (the FIFO `acquire` a job awaits once running).
    write_permit: Arc<tokio::sync::Semaphore>,
    /// The in-process background-job table (§3 `serve/jobs/`), read by the
    /// `/api/v1/jobs*` routes and written by every job worker.
    jobs: Arc<jobs::Registry>,
    /// Canonicalized `--allow-path <dir>` entries — one source feeding the
    /// `contain_abs` allowed-roots set (§Security "Path containment").
    allow_path: Arc<Vec<PathBuf>>,
    /// The server process's own git work-tree root (canonicalized), when its
    /// cwd is inside one — the bootstrap case for a fresh install with no
    /// `repo_marker` rows yet.
    bootstrap_root: Option<Arc<PathBuf>>,
}

impl AppState {
    /// Open `comemory.db` at `paths`, mint a fresh per-session bearer token,
    /// and assemble the shared handler state for one serve session.
    /// Ensures the data-dir tree exists first (`Paths::ensure_dirs`) so
    /// every route — including read-only ones — can rely on it existing.
    /// [`serve`] calls this to build its live state; an in-process test
    /// harness (`tests/common/serve_state.rs`) calls it directly and hands
    /// the result to [`router::build_router`], skipping the socket bind.
    pub fn new(paths: &Paths, opts: ServeOptions) -> Result<Self> {
        paths.ensure_dirs()?;
        let conn = connection::open(paths.db_path())?;
        let token = security::generate_token()?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            paths: Arc::new(paths.clone()),
            roots: Arc::new(opts.roots),
            token: Arc::from(token.as_str()),
            read_only: opts.read_only,
            repo: opts.repo,
            cfg: Arc::new(Mutex::new(Arc::new(opts.cfg))),
            embed_cmd: opts.embed_cmd.map(Arc::from),
            write_permit: Arc::new(tokio::sync::Semaphore::new(1)),
            jobs: Arc::new(jobs::Registry::new()),
            allow_path: Arc::new(opts.allow_path),
            bootstrap_root: discover_bootstrap_root().map(Arc::new),
        })
    }

    /// Lock the shared connection. Maps lock poisoning (a panic in another
    /// handler while holding the guard) to an internal error rather than
    /// propagating the panic.
    pub(crate) fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| Error::Other("serve: database lock poisoned".into()))
    }

    /// Reopen `comemory.db` at `paths` and swap it into the shared
    /// connection in place — called by the rebuild job right after it
    /// renames a freshly built DB over the live path (§Concurrency "Rebuild
    /// connection swap", AC-16). In-flight requests holding the OLD guard
    /// finish on the old (now-unlinked) inode; every later
    /// [`AppState::conn`] call sees the new DB.
    ///
    /// A reopen failure is propagated as-is and the mutex keeps the stale
    /// connection: the server then serves stale reads until restart — it
    /// never panics, never retries, and never poisons the lock.
    pub(crate) fn swap_conn(&self, paths: &Paths) -> Result<()> {
        // Open first, lock second: a failed open must leave the shared
        // connection exactly as it was, still usable by later requests.
        let fresh = connection::open(paths.db_path())?;
        *self.conn()? = fresh;
        Ok(())
    }

    /// Reload `config.toml` and swap it into the shared `cfg` slot — called
    /// by a `tune`/`bandit` apply job immediately after it rewrites the
    /// file (§Route-map Notes). Every later [`AppState::cfg`] call sees the
    /// new knobs without a restart. A reload failure is propagated as-is
    /// and the slot keeps the PRIOR config (documented: HTTP ranking then
    /// keeps using the pre-apply knobs until the next successful reload or
    /// a restart — it never panics). Goes through the same layered
    /// defaults→file→env resolution as a fresh process start
    /// ([`crate::cli::load_config`]), not a hand-rolled TOML read.
    pub(crate) fn reload_cfg(&self, paths: &Paths) -> Result<()> {
        // Load first, lock second: a failed reload must leave the shared
        // config exactly as it was, still usable by later requests.
        let fresh = crate::cli::load_config(paths)?;
        *self
            .cfg
            .lock()
            .map_err(|_| Error::Other("serve: config lock poisoned".into()))? = Arc::new(fresh);
        Ok(())
    }

    /// The data-dir layout (`Paths`) this session was started with.
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// The per-session bearer token, required on every `/api/*` request
    /// (`X-Comemory-Token` or `Authorization: Bearer` header, `?token=`
    /// query, or a `comemory_token` cookie). Exposed so [`serve`] can print
    /// it in the startup banner and an in-process test harness
    /// (`tests/common/serve_state.rs`) can authenticate requests driven
    /// straight through [`router::build_router`] without binding a socket.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Whether writes are disabled for this session.
    pub(crate) fn read_only(&self) -> bool {
        self.read_only
    }

    /// The session-wide default repo scope (`--repo`), if any.
    pub(crate) fn repo(&self) -> Option<&str> {
        self.repo.as_deref()
    }

    /// The `--root <repo>=<path>` overrides, for the routes that resolve a
    /// repo label to a working-tree root on the server's behalf (`POST
    /// /memories/{id}/references/refresh`) — the same map
    /// [`AppState::allowed_roots`] folds into the containment set.
    pub(crate) fn roots(&self) -> &RootOverrides {
        &self.roots
    }

    /// The current layered config, cloned out of the swappable slot. A
    /// poisoned lock (a panic in another handler mid-swap) is recovered
    /// rather than propagated: the only mutation is a single `Arc` pointer
    /// swap, so a poisoned guard's inner value is never torn.
    pub(crate) fn cfg(&self) -> Arc<Config> {
        self.cfg.lock().map_or_else(
            |poisoned| Arc::clone(&poisoned.into_inner()),
            |g| Arc::clone(&g),
        )
    }

    /// The embed command for the server-side vectorizing routes, if
    /// configured.
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

    /// The background-job registry: the `/api/v1/jobs*` routes read it, and
    /// `jobs::spawn_job` takes it (with [`AppState::write_permit`]) to run a
    /// long command off the request path.
    pub fn jobs(&self) -> &Arc<jobs::Registry> {
        &self.jobs
    }

    /// The full allowed-roots set for `security::contain_abs`: `--root`
    /// override paths (canonicalized here — the map's values are not
    /// guaranteed pre-canonical) ∪ every stored `repo_marker.root_path` ∪
    /// the server-cwd bootstrap root, if any ∪ `--allow-path` entries.
    /// Deduplicated. Best-effort against `conn`: a `repo_marker` read
    /// failure is logged and drops only that source, not the other three —
    /// it is an enrichment, not a hard requirement. Called from
    /// `serve::routes::code`, `sources`, `learning`, and `maint::admin`
    /// before every containment check ahead of a mutating filesystem op.
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
/// listener, print the base URL and the session token, and serve until the
/// process is interrupted.
pub async fn serve(paths: &Paths, opts: ServeOptions, json: bool) -> Result<()> {
    // `port` is read off `opts` before it moves into `AppState::new` (which
    // also hoists `ensure_dirs()` — so every route, including read-only
    // ones like `GET /doctor`, can rely on the data-dir tree already
    // existing, with no per-request side effect).
    let port = opts.port;
    let read_only = opts.read_only;
    let state = AppState::new(paths, opts)?;
    let token = state.token().to_string();

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(Error::Io)?;
    let port = listener.local_addr().map_err(Error::Io)?.port();
    let url = format!("http://127.0.0.1:{port}/api/v1");
    emit_banner(&url, port, &token, read_only, json)?;

    let app = router::build_router(state);
    axum::serve(listener, app).await.map_err(Error::Io)?;
    Ok(())
}

/// Print the base URL and token to stdout. Uses the `output` module (not
/// `tracing`, which is silent without `RUST_LOG`) so both are always visible
/// and machine-readable under `--json`.
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
    crate::output::tty::header(&format!("comemory serve{mode} → {url}  token={token}"))
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
