//! `comemory sources` — list registered document sources with per-status
//! file counts.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::output::json;
use crate::prelude::*;
use crate::source::mirror;
use crate::source::registry::Registry;
use crate::store::{connection, sources};

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

/// One `comemory sources` row.
#[derive(Serialize, Debug)]
struct Row {
    id: String,
    canonical_path: String,
    kind: String,
    repo: Option<String>,
    status: String,
    indexed: usize,
    error: usize,
    stale: usize,
    last_checked: String,
}

/// List every registered source, reconciling the SQLite mirror against
/// `sources.toml` first so the report reflects the durable source of
/// truth even when the mirror fell behind (e.g. a `sources.toml` edited
/// by hand, or a run of `comemory index`/`unindex` from another process).
pub async fn run(_a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let registry = Registry::new(paths.clone());
    let conn = connection::open(paths.db_path())?;
    mirror::reconcile(&conn, &registry.load()?)?;

    let mut rows = Vec::new();
    for root in sources::list(&conn)? {
        let counts = sources::file_status_counts(&conn, &root.id)?;
        rows.push(Row {
            id: root.id,
            canonical_path: root.canonical_path,
            kind: root.kind,
            repo: root.repo,
            status: root.status,
            indexed: counts.indexed,
            error: counts.error,
            stale: counts.stale,
            last_checked: root.updated_at,
        });
    }
    emit(json_flag, &rows)
}

/// Emit the source listing: a JSON array under `--json`, else one line
/// per source (or an explicit empty-state line).
fn emit(json_flag: bool, rows: &[Row]) -> Result<()> {
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
