//! `comemory sources` — list registered document sources with per-status
//! file counts. The reconcile-then-list middle lives in `api::sources`
//! (Binding Rule 1); the CLI always reconciles (`reconcile: true`).

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Human-readable table
  comemory sources

  # Machine-readable, for scripting
  comemory sources --json";

/// Arguments to `comemory sources`. No subcommand-local flags today;
/// wrapped in a struct so future opt-in flags can land without breaking
/// the dispatcher signature.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args;

/// List every registered source, delegating the reconcile-then-list middle
/// to `api::sources::run`.
pub async fn run(_a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;

    let req = api::sources::Request { reconcile: true };
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let rows = api::sources::run(&mut ctx, req)?;
    emit(json_flag, &rows)
}

/// Emit the source listing: a JSON array under `--json`, else one line
/// per source (or an explicit empty-state line).
fn emit(json_flag: bool, rows: &[api::sources::Row]) -> Result<()> {
    if json_flag {
        json::write(&rows)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    if rows.is_empty() {
        writeln!(out, "no sources registered")?;
        return Ok(());
    }
    for r in rows {
        writeln!(
            out,
            "{}  {}  {}  repo={}  status={}  indexed={} error={} stale={}  checked={}",
            r.id,
            r.kind,
            r.canonical_path,
            r.repo.as_deref().unwrap_or("-"),
            r.status,
            r.indexed,
            r.error,
            r.stale,
            r.last_checked,
        )?;
    }
    Ok(())
}
