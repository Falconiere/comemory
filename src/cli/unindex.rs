//! `comemory unindex <SOURCE_ID|PATH>` — unregister a document source and
//! delete its derived rows. External files under the source root are
//! never touched (AC-6).

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::config::Config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Unregister by source id (see `comemory sources`)
  comemory unindex 3f9c2a1b4e5d6f708192a3b4c5d6e7f8

  # Unregister by the path it was registered under
  comemory unindex ~/notes";

/// Arguments to `comemory unindex`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// The source id (see `comemory sources`) or the path it was
    /// registered under.
    pub target: String,
}

/// Unregister the source matching `a.target` and delete its derived rows
/// (`source_files`/`documents`/`document_chunks`/`document_fts`) plus the
/// `member_of_source`/`references_document` edges those rows own.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let cfg = Config::defaults();

    let req = api::unindex::Request { target: a.target };
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let output = api::unindex::run(&mut ctx, req)?;
    emit(json_flag, &output)
}

/// Emit the unindex report: a JSON object under `--json`, else a two-line
/// TTY summary.
fn emit(json_flag: bool, output: &api::unindex::Response) -> Result<()> {
    if json_flag {
        json::write(output)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "unregistered {} ({})",
        output.source_id, output.canonical_path
    )?;
    writeln!(out, "  documents removed: {}", output.documents_removed)?;
    Ok(())
}
