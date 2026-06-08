//! `comemory doctor` — runtime health check against the v0.2 SQLite
//! storage stack.
//!
//! v0.2 replaces the v0.1 "count memories on disk" report with a
//! sanity sweep over `comemory.db`: data directory, whether the DB
//! file is writable, the applied schema version, and whether the
//! `sqlite-vec` extension was loaded into the open connection.
//!
//! The report is intentionally narrow — anything richer (per-table
//! counts, last-failure ledger, model availability) belongs in a
//! follow-up command so `doctor` stays cheap and always-on.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::{load_config, resolve_data_dir};
use crate::config::paths::Paths;
use crate::output::json;
use crate::prelude::*;
use crate::store::{connection, migrate};

/// Example invocations shown at the bottom of `comemory doctor --help`.
pub const EXAMPLES: &str = "\
Examples:
  # Human-readable health report
  comemory doctor

  # JSON for monitoring or CI
  comemory doctor --json";

/// Arguments to `comemory doctor`. No subcommand-local flags today;
/// wrapped in a struct so future opt-in flags can land without breaking
/// the dispatcher signature.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args;

/// JSON shape emitted under `--json` and used to compute TTY output.
#[derive(Serialize, Debug)]
pub struct Report {
    /// Resolved data directory (after `--data-dir` / `COMEMORY_DATA_DIR`
    /// fallback).
    pub data_dir: String,
    /// `true` when `comemory.db` exists and is writable.
    pub db_writable: bool,
    /// Applied schema version from `schema_meta.version` (currently `"3"`).
    pub schema_version: String,
    /// `true` when `vec_version()` returns a string, i.e. the
    /// sqlite-vec extension was loaded into this connection.
    pub sqlite_vec_loaded: bool,
    /// Free-form identifier of the embedder the operator configured
    /// (e.g. `ollama:nomic-embed-text`). `None` when `COMEMORY_EMBED_HINT`
    /// is not set.
    pub embed_hint: Option<String>,
}

/// Build and emit the doctor report.
pub async fn run(_args: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;

    let db_path = paths.db_path();
    let writable = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&db_path)
        .is_ok();
    let conn = connection::open(&db_path)?;
    let schema_version: String = conn.query_row(
        "SELECT value FROM schema_meta WHERE key = 'version'",
        [],
        |r| r.get(0),
    )?;
    let sqlite_vec_loaded = conn
        .query_row("SELECT vec_version()", [], |r| r.get::<_, String>(0))
        .is_ok();
    let cfg = load_config(&paths)?;
    let report = Report {
        data_dir: paths.data_dir().to_string_lossy().into_owned(),
        db_writable: writable,
        schema_version,
        sqlite_vec_loaded,
        embed_hint: cfg.embed_hint,
    };
    if report.schema_version != migrate::CURRENT_VERSION {
        return Err(Error::Migration(format!(
            "schema version {} != expected {}",
            report.schema_version,
            migrate::CURRENT_VERSION
        )));
    }
    emit(&report, json_flag)
}

/// Write the doctor report to stdout. JSON mode serialises the
/// `Report` struct verbatim; TTY mode renders a 4-line summary.
fn emit(report: &Report, json_flag: bool) -> Result<()> {
    if json_flag {
        json::write(report)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "data_dir          : {}", report.data_dir)?;
    writeln!(out, "db_writable       : {}", report.db_writable)?;
    writeln!(out, "schema_version    : {}", report.schema_version)?;
    writeln!(out, "sqlite_vec_loaded : {}", report.sqlite_vec_loaded)?;
    writeln!(
        out,
        "embed_hint        : {}",
        report.embed_hint.as_deref().unwrap_or("(none)")
    )?;
    Ok(())
}
