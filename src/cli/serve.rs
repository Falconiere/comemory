//! `comemory serve` — launch the loopback `/api/v1` HTTP server.
//!
//! Binds an axum server to `127.0.0.1` (ephemeral port by default), serving
//! the versioned REST surface over `comemory.db` and the indexed source
//! tree for the console and any local agent or script. Every request is
//! gated by a per-session token and a loopback Host guard, and every
//! path-taking mutating route by a containment check; `--read-only`
//! refuses every mutating route.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::prelude::*;
use crate::serve::{self, RootOverrides, ServeOptions};

const EXAMPLES: &str = "\
Examples:
  # Serve the /api/v1 surface for every indexed repo on an ephemeral port
  comemory serve

  # Default every read to one repo, on a fixed port
  comemory serve --repo myrepo --port 8787

  # Read-only: every mutating route answers 405
  comemory serve --read-only

  # Supply a repo root for repos indexed before the v7 schema captured it
  comemory serve --root myrepo=/abs/path/to/repo

  # Allow a mutating route to touch an extra filesystem path (e.g. eval's
  # --golden file) outside any indexed repo root
  comemory serve --allow-path /abs/path/to/golden-dir";

/// Arguments to `comemory serve`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Default repo label (as passed to `index-code --repo`) for every read
    /// that accepts a `repo` filter. An explicit `repo` parameter or an
    /// `X-Comemory-Repo` header on the request overrides it.
    #[arg(long)]
    pub repo: Option<String>,
    /// Loopback port to bind. `0` (default) selects an ephemeral port whose
    /// URL is printed at startup.
    #[arg(long, default_value_t = 0)]
    pub port: u16,
    /// Refuse every mutating /api/v1 route with 405 read_only.
    #[arg(long, default_value_t = false)]
    pub read_only: bool,
    /// Override a repo's working-tree root as `<repo>=<abs-path>` (repeatable).
    /// Required for repos indexed before the v7 schema captured the root.
    #[arg(long = "root", value_name = "REPO=PATH")]
    pub root: Vec<String>,
    /// Embed command for the routes that vectorize on the server's behalf
    /// (POST /api/v1/doctor/reembed); run as sh -c, reads text on stdin,
    /// emits {"embedding":[..]}. Unset → those routes answer 503. Mirrors
    /// COMEMORY_EMBED_CMD.
    #[arg(long, value_name = "CMD", env = "COMEMORY_EMBED_CMD")]
    pub embed_cmd: Option<String>,
    /// Allow a path-taking mutating route (`index-code`, `ast --file`,
    /// `install-hooks --repo`, `eval`/`tune`/`bandit --golden`) to touch a
    /// filesystem path under this directory, on top of `--root` overrides
    /// and the stored `repo_marker` roots (repeatable).
    #[arg(long = "allow-path", value_name = "DIR")]
    pub allow_path: Vec<PathBuf>,
}

/// Parse, validate, and launch the server. Blocks until interrupted.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    // `serve::serve` hoists `ensure_dirs()` to its own startup (so it also
    // covers callers that invoke it directly, e.g. tests) — `load_config`
    // itself tolerates a missing config file/dir.
    let cfg = load_config(&paths)?;
    let roots = parse_roots(&a.root)?;
    let allow_path = parse_allow_paths(&a.allow_path)?;
    let opts = ServeOptions {
        repo: a.repo,
        port: a.port,
        read_only: a.read_only,
        roots,
        cfg,
        embed_cmd: a.embed_cmd,
        allow_path,
    };
    serve::serve(&paths, opts, json).await
}

/// Canonicalize each `--allow-path <dir>` entry. A non-existent or
/// inaccessible entry fails startup with `Error::Config` — unlike
/// `repo_marker_roots::all_roots`'s best-effort skip, this is explicit
/// user-supplied config, so a typo must surface loudly rather than being
/// silently dropped from the allowed-roots set.
fn parse_allow_paths(raw: &[PathBuf]) -> Result<Vec<PathBuf>> {
    raw.iter()
        .map(|p| {
            p.canonicalize().map_err(|e| {
                Error::Config(format!("--allow-path `{}` is unusable: {e}", p.display()))
            })
        })
        .collect()
}

/// Parse `--root <repo>=<path>` flags into a [`RootOverrides`] map. Splits on
/// the first `=` so paths containing `=` survive; rejects entries missing the
/// separator or with an empty repo label.
fn parse_roots(raw: &[String]) -> Result<RootOverrides> {
    let mut map = RootOverrides::new();
    for entry in raw {
        let (repo, path) = entry
            .split_once('=')
            .ok_or_else(|| Error::Config(format!("--root must be <repo>=<path>, got `{entry}`")))?;
        if repo.is_empty() || path.is_empty() {
            return Err(Error::Config(format!(
                "--root must be <repo>=<path> with both sides non-empty, got `{entry}`"
            )));
        }
        map.insert(repo.to_string(), PathBuf::from(path));
    }
    Ok(map)
}
