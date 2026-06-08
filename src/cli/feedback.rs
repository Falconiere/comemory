//! `comemory feedback` — record per-memory used/irrelevant feedback into the
//! SQLite stats database. Accepts comma-separated id lists for each side.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::output::json;
use crate::prelude::*;
use crate::stats::feedback::Feedback;
use crate::stats::sqlite::StatsDb;

const EXAMPLES: &str = "\
Examples:
  # Mark two hits as useful and one as irrelevant
  comemory feedback q-2026-05-17-001 --used a1b2c3d4,e5f6a7b8 --irrelevant 0011223344

  # Only-used feedback
  comemory feedback q-2026-05-17-002 --used a1b2c3d4

  # Only-irrelevant feedback
  comemory feedback q-2026-05-17-003 --irrelevant 0011223344";

/// Arguments to `comemory feedback`. `query_id` is captured for future provenance;
/// it is accepted today but does not yet influence storage.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
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

/// Record feedback for each id provided and emit a one-line ack (or a JSON
/// envelope with the recorded counts when `json` is set).
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut db = StatsDb::open(paths.stats_db())?;
    let fb = Feedback::new(&mut db);
    let mut used = 0usize;
    let mut irrelevant = 0usize;
    // Trim whitespace around each comma-split id so `--used "a, b,c"` records
    // `a`, `b`, `c` rather than the literal strings ` b` and `c` (the latter
    // would silently miss every downstream lookup keyed on the bare id).
    for id in a.used.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        fb.record_used(id)?;
        used += 1;
    }
    for id in a
        .irrelevant
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        fb.record_irrelevant(id)?;
        irrelevant += 1;
    }
    if json_flag {
        json::write(&serde_json::json!({
            "ok": true,
            "used": used,
            "irrelevant": irrelevant,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "ok")?;
    }
    Ok(())
}
