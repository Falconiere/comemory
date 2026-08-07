//! `comemory doctor` — runtime health check against the v0.2 SQLite
//! storage stack: data directory, DB writability, schema version, and
//! whether `sqlite-vec` loaded. The probe lives in `api::doctor` (Binding
//! Rule 1); this wrapper resolves paths/config and renders the report.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;

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

/// Build and emit the doctor report via `api::doctor::run`. Uses
/// `Ctx::lazy` (never `Ctx::borrowed` with an eagerly-opened connection) so
/// the careful non-creating probe inside `api::doctor` stays in control of
/// whether a connection ever gets opened.
pub async fn run(_args: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let report = api::doctor::run(&mut ctx, api::doctor::Request {})?;
    emit(&report, json_flag)
}

/// Write the doctor report to stdout. JSON mode serialises the
/// `Report` struct verbatim; TTY mode renders a 4-line summary.
fn emit(report: &api::doctor::Report, json_flag: bool) -> Result<()> {
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
