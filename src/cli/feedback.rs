//! `qwick feedback` — record per-memory used/irrelevant feedback into the
//! SQLite stats database. Accepts comma-separated id lists for each side.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::prelude::*;
use crate::stats::feedback::Feedback;
use crate::stats::sqlite::StatsDb;

/// Arguments to `qwick feedback`. `query_id` is captured for future provenance;
/// it is accepted today but does not yet influence storage.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Identifier of the originating search query (recorded for provenance).
    pub query_id: String,
    /// Comma-separated memory ids that were used.
    #[arg(long, default_value = "")]
    pub used: String,
    /// Comma-separated memory ids that were judged irrelevant.
    #[arg(long, default_value = "")]
    pub irrelevant: String,
}

/// Record feedback for each id provided and emit a one-line ack.
pub async fn run(a: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut db = StatsDb::open(paths.stats_db())?;
    let fb = Feedback::new(&mut db);
    for id in a.used.split(',').filter(|s| !s.is_empty()) {
        fb.record_used(id)?;
    }
    for id in a.irrelevant.split(',').filter(|s| !s.is_empty()) {
        fb.record_irrelevant(id)?;
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "ok")?;
    Ok(())
}
