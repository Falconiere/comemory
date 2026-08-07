//! `comemory index <PATH>...` — register one or more files or directories
//! as document sources and run their synchronous initial reconcile. The
//! shared middle (registration + reconcile loop) lives in `api::index`
//! (Binding Rule 1); this file keeps arg-parsing, the `--strict` exit-code
//! decision, and TTY/JSON rendering.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::index::Output;
use crate::api::{self, Ctx};
use crate::cli::{load_config, resolve_data_dir};
use crate::config::paths::Paths;
use crate::output::{json, tty};
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Register and index a directory of notes
  comemory index ~/notes

  # Register a single file with an explicit repo label
  comemory index README.md --repo comemory

  # Register two sources in one run, failing loudly on any per-file error
  comemory index ~/notes ~/docs --strict";

/// Arguments to `comemory index`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// One or more files or directories to register as document sources.
    #[arg(required = true, value_name = "PATH")]
    pub path: Vec<PathBuf>,
    /// Repository label attached to every document under each registered
    /// source. Defaults to the basename of the nearest enclosing git
    /// worktree, when one exists.
    #[arg(long)]
    pub repo: Option<String>,
    /// Exit `65` (`EX_DATAERR`) when any per-file error (corrupt/unreadable/
    /// oversized) occurred, after every file has been attempted. Without
    /// this flag a partial-success reconcile still exits `0`.
    #[arg(long, default_value_t = false)]
    pub strict: bool,
}

/// Register every `a.path` entry and run its synchronous reconcile via
/// [`api::index::run`]. See that module's doc for why `--strict` is
/// re-checked here rather than inside the shared middle.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let cfg = load_config(&paths)?;
    let mut conn = connection::open(paths.db_path())?;
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::index::Request {
        path: a
            .path
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        repo: a.repo.clone(),
        strict: a.strict,
    };
    let output = api::index::run(&mut ctx, req)?;

    let strict_failed = a.strict && output.sources.iter().any(|r| !r.errors.is_empty());
    emit(json_flag, &output)?;
    if strict_failed {
        return Err(Error::Document(
            "index --strict: one or more files failed during reconcile".into(),
        ));
    }
    Ok(())
}

/// Emit the reconcile report: a single JSON object under `--json`, else a
/// per-source summary line plus a `warning:` line per diagnostic.
fn emit(json_flag: bool, output: &Output) -> Result<()> {
    if json_flag {
        json::write(output)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    for r in &output.sources {
        writeln!(out, "source {} ({})", r.source_id, r.canonical_path)?;
        writeln!(
            out,
            "  indexed={} unchanged={} too_large={} unsupported={} removed={}",
            r.indexed, r.unchanged, r.too_large, r.unsupported, r.removed
        )?;
        for e in &r.errors {
            tty::warning(&format!("{}: {} ({})", e.path, e.message, e.kind))?;
        }
    }
    Ok(())
}
