//! `comemory install-hooks` — drop git hooks into a repo so
//! commits/merges/checkouts kick off `comemory index-code` in the
//! background. The hook-writing middle lives in `domains::code::install_hooks`
//! (Binding Rule 1).

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::config::Config;
use crate::output::json;
use crate::prelude::*;
use crate::utilities::context::Ctx;

const EXAMPLES: &str = "\
Examples:
  # Install into the current repo
  comemory install-hooks

  # Install into a specific repo path
  comemory install-hooks --repo /path/to/repo

  # Overwrite a hand-written hook (comemory's own is refreshed anyway)
  comemory install-hooks --force";

/// Arguments to `comemory install-hooks`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Repo root to install hooks into. Defaults to the current working
    /// directory.
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,
    /// Overwrite a hook comemory did not write. A hook comemory DID write is
    /// always refreshed to this binary's body, with or without this flag, so
    /// one installed by an older release stops labelling every `git worktree`
    /// as its own repo. Without this flag the command refuses to clobber a
    /// hand-written `post-commit`/`post-merge`/`post-checkout`.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

/// Install the three reindex hooks via `crate::domains::code::install_hooks::run`, refreshing
/// any comemory already wrote and clobbering a foreign one only with
/// `--force`. On success the human-readable line lists the
/// hooks that were written; under `--json` we emit a small object so callers
/// can detect success programmatically. `install-hooks` has no `Paths`/db
/// dependency, so `data_dir` resolves a throwaway `Ctx::lazy` that is never
/// opened.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = crate::config::Paths::new(crate::config::paths::resolve_data_dir(data_dir));
    let cfg = Config::defaults();
    let req = crate::domains::code::install_hooks::Request {
        repo: a.repo.display().to_string(),
        force: a.force,
    };
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = crate::domains::code::install_hooks::run(&mut ctx, req)?;
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
