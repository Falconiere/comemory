//! `comemory serve` — launch the local web viewer + in-browser code editor.
//!
//! Binds an axum server to `127.0.0.1` (ephemeral port by default), serving
//! the embedded React/Vite SPA and a small JSON/file API over `comemory.db`
//! and the indexed source tree. Reads and writes are gated by a per-session
//! token, a loopback Host guard, and a path-containment check; `--read-only`
//! disables writes entirely. This complements — it does not replace — the
//! static `comemory graph --format html` export.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::prelude::*;
use crate::serve::{self, RootOverrides, ServeOptions};

const EXAMPLES: &str = "\
Examples:
  # Serve the graph + editor for every indexed repo on an ephemeral port
  comemory serve

  # One repo, fixed port, opened in the browser
  comemory serve --repo myrepo --port 8787 --open

  # Read-only exploration (no writes to disk)
  comemory serve --read-only

  # Supply a repo root for repos indexed before the v7 schema captured it
  comemory serve --root myrepo=/abs/path/to/repo";

/// Arguments to `comemory serve`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Restrict the graph to one repo label (as passed to `index-code --repo`).
    #[arg(long)]
    pub repo: Option<String>,
    /// Loopback port to bind. `0` (default) selects an ephemeral port whose
    /// URL is printed at startup.
    #[arg(long, default_value_t = 0)]
    pub port: u16,
    /// Disable all writes: `PUT /api/file` returns 405 and the editor's Save
    /// action is hidden.
    #[arg(long, default_value_t = false)]
    pub read_only: bool,
    /// Override a repo's working-tree root as `<repo>=<abs-path>` (repeatable).
    /// Required for repos indexed before the v7 schema captured the root.
    #[arg(long = "root", value_name = "REPO=PATH")]
    pub root: Vec<String>,
    /// Open the printed URL in the default browser after binding.
    #[arg(long, default_value_t = false)]
    pub open: bool,
}

/// Parse, validate, and launch the server. Blocks until interrupted.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let roots = parse_roots(&a.root)?;
    let opts = ServeOptions {
        repo: a.repo,
        port: a.port,
        read_only: a.read_only,
        roots,
        open: a.open,
    };
    serve::serve(&paths, opts, json).await
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
