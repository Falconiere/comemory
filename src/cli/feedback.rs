//! `comemory feedback` — record per-memory used/irrelevant feedback into the
//! SQLite stats database. Accepts comma-separated id lists for each side.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::{parse_id_csv, resolve_data_dir};
use crate::config::paths::Paths;
use crate::output::json;
use crate::prelude::*;
use crate::stats::feedback::Feedback;
use crate::stats::sqlite::StatsDb;

const EXAMPLES: &str = "\
Examples:
  # Mark two hits as useful and one as irrelevant
  comemory feedback q-2026-05-17-001 --used a1b2c3d4,e5f6a7b8 --irrelevant 00112233

  # Only-used feedback
  comemory feedback q-2026-05-17-002 --used a1b2c3d4

  # Only-irrelevant feedback
  comemory feedback q-2026-05-17-003 --irrelevant 00112233";

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
///
/// Both id lists go through the shared [`parse_id_csv`]: entries are
/// trimmed, de-duplicated (so `--used a,a` cannot double-count and skew
/// the Beta-feedback posterior), and validated as 8-hex memory ids (so a
/// typo'd id errors loudly instead of writing an orphan feedback row that
/// no ranking lookup will ever join).
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    // Validate BOTH lists before recording anything so a bad id in
    // `--irrelevant` cannot leave the `--used` half already committed.
    let used_ids = parse_id_csv(&a.used, "--used")?;
    let irrelevant_ids = parse_id_csv(&a.irrelevant, "--irrelevant")?;

    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut db = StatsDb::open(paths.stats_db())?;
    let fb = Feedback::new(&mut db);
    for id in &used_ids {
        fb.record_used(id)?;
    }
    for id in &irrelevant_ids {
        fb.record_irrelevant(id)?;
    }
    if json_flag {
        json::write(&serde_json::json!({
            "ok": true,
            "used": used_ids.len(),
            "irrelevant": irrelevant_ids.len(),
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "ok")?;
    }
    Ok(())
}
