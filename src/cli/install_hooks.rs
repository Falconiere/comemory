//! `comemory install-hooks` — drop git hooks into a repo so
//! commits/merges/checkouts kick off `comemory index-code` in the
//! background. The hook-writing middle lives in `api::install_hooks`
//! (Binding Rule 1).

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::config::Config;
use crate::output::json;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Install into the current repo
  comemory install-hooks

  # Install into a specific repo path
  comemory install-hooks --repo /path/to/repo

  # Overwrite any hand-written hooks
  comemory install-hooks --force";

/// Arguments to `comemory install-hooks`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Repo root to install hooks into. Defaults to the current working
    /// directory.
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,
    /// Overwrite existing hook files. Without this flag the command refuses
    /// to clobber a pre-existing `post-commit`/`post-merge`/`post-checkout`
    /// to avoid surprising users with hand-written hooks.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

/// Install (or, with `--force`, overwrite) the three reindex hooks via
/// `api::install_hooks::run`. On success the human-readable line lists the
/// hooks that were written; under `--json` we emit a small object so callers
/// can detect success programmatically. `install-hooks` has no `Paths`/db
/// dependency, so `data_dir` resolves a throwaway `Ctx::lazy` that is never
/// opened.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = crate::config::Paths::new(crate::cli::resolve_data_dir(data_dir));
    let cfg = Config::defaults();
    let req = api::install_hooks::Request {
        repo: a.repo.display().to_string(),
        force: a.force,
    };
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = api::install_hooks::run(&mut ctx, req)?;
    if json_flag {
        json::write(&resp)?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "installed {} hooks in {}",
            resp.installed.join(", "),
            resp.repo
        )?;
    }
    Ok(())
}
