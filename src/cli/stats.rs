//! `comemory stats` — corpus counters and database size in one call.
//!
//! The counting itself lives in `api::stats` (Binding Rule 1), including
//! the must-not-create-the-db-on-a-fresh-dir invariant this command shares
//! with `gc`.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;

/// Example invocations shown at the bottom of `comemory stats --help`.
pub const EXAMPLES: &str = "\
Examples:
  # Corpus counters and database size
  comemory stats

  # Scope the per-repo counters to one repo (db_bytes stays global)
  comemory stats --repo comemory

  # JSON for a dashboard or CI
  comemory stats --json";

/// Arguments to `comemory stats`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Scope the per-repo counters (memories, trashed, code symbols,
    /// documents) to one repo label. Edge, database-size, repo, and
    /// markdown counts stay global.
    #[arg(long)]
    pub repo: Option<String>,
}

/// Report corpus counters. On a data dir with no `comemory.db` the
/// SQL-backed counters are zero and `schema_version` is `unknown` — the
/// database is never created as a side effect of asking how big it is.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = api::stats::run(&mut ctx, api::stats::Request { repo: a.repo })?;

    if json_flag {
        json::write(&resp)?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "memories      {}", resp.memories)?;
        writeln!(out, "trashed       {}", resp.trashed)?;
        writeln!(out, "markdown      {}", resp.markdown_files)?;
        writeln!(out, "code symbols  {}", resp.code_symbols)?;
        writeln!(out, "documents     {}", resp.documents)?;
        writeln!(out, "graph edges   {}", resp.edges)?;
        writeln!(out, "repos         {}", resp.repos)?;
        writeln!(out, "comemory.db   {}", human_bytes(resp.db_bytes))?;
        writeln!(out, "schema        v{}", resp.schema_version)?;
    }
    Ok(())
}

/// Render a byte count the way the TTY view shows it. JSON keeps the raw
/// integer — this is presentation only.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
