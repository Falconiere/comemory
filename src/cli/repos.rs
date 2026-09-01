//! `comemory repos` — the indexed code-repository inventory: per-repo file
//! and symbol counts plus git freshness against the working tree on disk.
//! The `repo_marker` join and git-state resolution live in `api::repos`
//! (Binding Rule 1).

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;

/// Example invocations shown at the bottom of `comemory repos --help`.
pub const EXAMPLES: &str = "\
Examples:
  # Every indexed repo, ordered by label
  comemory repos

  # Narrow to one repo label
  comemory repos --repo comemory

  # JSON for the console or scripting
  comemory repos --json";

/// Arguments to `comemory repos`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Narrow the inventory to one repo label.
    #[arg(long)]
    pub repo: Option<String>,
}

/// List every indexed repo's inventory row via `api::repos::run`. Uses
/// `Ctx::lazy` (never eagerly opens `comemory.db`) so a data dir with no
/// database yet reports an empty inventory rather than creating one.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = api::repos::run(&mut ctx, api::repos::Request { repo: a.repo })?;
    emit(json_flag, &resp)
}

/// Emit the repo inventory: the whole `{"repos": [...]}` object under
/// `--json` (matching `api::repos::Response`'s shape verbatim, the same
/// contract `GET /api/v1/repos` serves), else an aligned table with a
/// `root=/remote=/last_head=/...` detail line under each row.
fn emit(json_flag: bool, resp: &api::repos::Response) -> Result<()> {
    if json_flag {
        json::write(resp)?;
        return Ok(());
    }
    let rows = &resp.repos;
    let mut out = std::io::stdout().lock();
    if rows.is_empty() {
        writeln!(out, "no repos indexed")?;
        return Ok(());
    }
    let repo_w = rows.iter().map(|r| r.repo.len()).max().unwrap_or(4).max(4);
    writeln!(
        out,
        "{:<repo_w$}  {:<7}  {:>5}  {:>7}  {:>8}  branch",
        "repo", "status", "files", "symbols", "memories"
    )?;
    for r in rows {
        writeln!(
            out,
            "{:<repo_w$}  {:<7}  {:>5}  {:>7}  {:>8}  {}",
            r.repo,
            r.status,
            r.files,
            r.symbols,
            r.memories,
            r.branch.as_deref().unwrap_or("-"),
        )?;
        writeln!(
            out,
            "  root={}  remote={}  last_head={}  last_indexed_at={}  changed_files={}",
            r.root_path.as_deref().unwrap_or("-"),
            r.remote.as_deref().unwrap_or("-"),
            r.last_head.as_deref().unwrap_or("-"),
            r.last_indexed_at.as_deref().unwrap_or("-"),
            r.changed_files.map_or("-".to_string(), |n| n.to_string()),
        )?;
    }
    Ok(())
}
