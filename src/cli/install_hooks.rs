//! `comemory install-hooks` — drop git hooks into a repo so every HEAD move
//! (commit, merge, checkout, rewrite) runs `comemory sync --action auto` for
//! it in the background. The hook-writing middle lives in `domains::code::install_hooks`
//! (Binding Rule 1).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::Args as ClapArgs;

use crate::cli::output::json;
use crate::config::Config;
use crate::domains::code::git_utils;
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
    /// hand-written `post-commit`/`post-merge`/`post-checkout`/`post-rewrite`.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

/// Install the reindex hooks via `crate::domains::code::install_hooks::run`,
/// refreshing any comemory already wrote and clobbering a foreign one only
/// with `--force`, then [`kick`] the first pass so the repo is indexed and
/// synced now rather than at its next commit. `--json` adds `kicked`.
/// `install-hooks` itself never opens the store: `data_dir` resolves a
/// `Ctx::lazy` that is never opened, and is handed to the kicked pass.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = crate::config::Paths::new(crate::config::paths::resolve_data_dir(data_dir));
    let cfg = Config::defaults();
    let req = crate::domains::code::install_hooks::Request {
        repo: a.repo.display().to_string(),
        force: a.force,
    };
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = crate::domains::code::install_hooks::run(&mut ctx, req)?;
    let kicked = kick(&a.repo, paths.data_dir());
    if json_flag {
        let mut report = serde_json::to_value(&resp)?;
        report["kicked"] = serde_json::Value::Bool(kicked);
        json::write(&report)?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "installed {} hooks in {}",
            resp.installed.join(", "),
            resp.repo
        )?;
        if kicked {
            writeln!(out, "first index + sync started in the background")?;
        }
    }
    Ok(())
}

/// Run the freshly written `post-commit` hook once, exactly as git would, so
/// the repo is registered, indexed and synced without waiting for a commit.
/// The hook backgrounds its pass and returns at once; `data_dir` is passed
/// down so the pass writes to the store this command was pointed at. `false`
/// when the hook could not be run — the install stands either way.
fn kick(repo: &Path, data_dir: &Path) -> bool {
    let hook = git_utils::hooks_dir(repo).join("post-commit");
    let status = Command::new(&hook)
        .current_dir(repo)
        .env("COMEMORY_DATA_DIR", data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(st) => st.success(),
        Err(e) => {
            tracing::debug!(hook = %hook.display(), error = %e, "install-hooks: kick failed");
            false
        }
    }
}
