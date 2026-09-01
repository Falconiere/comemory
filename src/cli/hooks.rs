//! `comemory hooks` — read and per-hook toggle the reindex git hooks
//! `install-hooks` writes, plus the config-backed search→edit
//! auto-reinforcement row. The read/write logic lives in `api::hooks`
//! (Binding Rule 1).

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;

/// Example invocations shown at the bottom of `comemory hooks --help`.
pub const EXAMPLES: &str = "\
Examples:
  # List every hook's state for the current repo
  comemory hooks

  # List for a specific repo
  comemory hooks --repo /path/to/repo

  # Turn one git hook off, leaving the other two untouched
  comemory hooks --disable post-checkout

  # Turn search->edit auto-reinforcement off
  comemory hooks --disable search-edit-reinforcement

  # JSON for the console or scripting
  comemory hooks --json";

/// Arguments to `comemory hooks`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Repo root the three git hooks are read from / written to. Defaults
    /// to the current working directory. Irrelevant to the
    /// `search-edit-reinforcement` row.
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,
    /// Install/enable one hook: `post-commit`, `post-merge`,
    /// `post-checkout`, or `search-edit-reinforcement`.
    #[arg(long, conflicts_with = "disable")]
    pub enable: Option<String>,
    /// Remove/disable one hook: `post-commit`, `post-merge`,
    /// `post-checkout`, or `search-edit-reinforcement`.
    #[arg(long, conflicts_with = "enable")]
    pub disable: Option<String>,
}

/// Report (and, with `--enable`/`--disable`, toggle) hook state via
/// `api::hooks::run`. Uses `Ctx::lazy` — this command never opens the
/// database.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let req = api::hooks::Request {
        repo: Some(a.repo.display().to_string()),
        enable: a.enable,
        disable: a.disable,
    };
    let resp = api::hooks::run(&mut ctx, req)?;
    emit(json_flag, &resp)
}

/// Emit the hook report: the whole `{"hooks": [...]}` object under
/// `--json` (matching `api::hooks::Response`'s shape verbatim, the same
/// contract `GET|POST /api/v1/hooks` serves), else an aligned
/// `name  installed  source` table.
fn emit(json_flag: bool, resp: &api::hooks::Response) -> Result<()> {
    if json_flag {
        json::write(resp)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    let name_w = resp
        .hooks
        .iter()
        .map(|h| h.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    writeln!(out, "{:<name_w$}  installed  source", "hook")?;
    for h in &resp.hooks {
        writeln!(out, "{:<name_w$}  {:<9}  {}", h.name, h.installed, h.source)?;
    }
    Ok(())
}
