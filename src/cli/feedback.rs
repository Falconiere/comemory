//! `comemory feedback` — record per-memory used/irrelevant feedback into the
//! SQLite stats database. Accepts comma-separated id lists for each side.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::{parse_id_csv, resolve_data_dir};
use crate::config::paths::Paths;
use crate::output::json;
use crate::prelude::*;
use crate::stats::feedback::{is_valid_query_id, record_with_provenance};
use crate::stats::sqlite::StatsDb;

const EXAMPLES: &str = "\
Examples:
  # Mark two hits as useful and one as irrelevant
  comemory feedback q-20260610-a1b2c3d4 --used a1b2c3d4,e5f6a7b8 --irrelevant 00112233

  # Only-used feedback
  comemory feedback q-20260610-b2c3d4e5 --used a1b2c3d4

  # Only-irrelevant feedback
  comemory feedback q-20260610-c3d4e5f6 --irrelevant 00112233";

/// Arguments to `comemory feedback`. `query_id` is the
/// `q-<yyyymmdd>-<8hex>` id printed by `comemory search`; each verdict is
/// recorded against it in `feedback_events`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Id of the originating search query (`q-<yyyymmdd>-<8hex>`, as
    /// printed by `comemory search`); recorded for provenance.
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
/// `query_id` must match the `q-<yyyymmdd>-<8hex>` shape printed by
/// `comemory search` — a typo'd id errors loudly instead of writing
/// provenance rows no `retrieval_log` join will ever find. A valid-shaped
/// id that is *absent* from `retrieval_log` (evicted by gc, or replayed
/// feedback) only warns: the verdicts are still recorded.
///
/// Both id lists go through the shared [`parse_id_csv`]: entries are
/// trimmed, de-duplicated (so `--used a,a` cannot double-count and skew
/// the Beta-feedback posterior), and validated as 8-hex memory ids (so a
/// typo'd id errors loudly instead of writing an orphan feedback row that
/// no ranking lookup will ever join).
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    if !is_valid_query_id(&a.query_id) {
        return Err(Error::Config(format!(
            "invalid query id `{}` (expected q-<yyyymmdd>-<8hex>, as printed by comemory search)",
            a.query_id
        )));
    }
    // Validate BOTH lists before recording anything so a bad id in
    // `--irrelevant` cannot leave the `--used` half already committed.
    let used_ids = parse_id_csv(&a.used, "--used")?;
    let irrelevant_ids = parse_id_csv(&a.irrelevant, "--irrelevant")?;

    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut db = StatsDb::open(paths.stats_db())?;
    let known: bool = db.conn().query_row(
        "SELECT EXISTS(SELECT 1 FROM retrieval_log WHERE query_id = ?1)",
        [&a.query_id],
        |r| r.get(0),
    )?;
    if !known {
        tracing::warn!(query_id = %a.query_id,
            "query id not found in retrieval_log (evicted or never logged); recording anyway");
    }
    record_with_provenance(&mut db, &a.query_id, &used_ids, &irrelevant_ids)?;
    if json_flag {
        json::write(&serde_json::json!({
            "ok": true,
            "used": used_ids.len(),
            "irrelevant": irrelevant_ids.len(),
            "query_id": a.query_id,
            "known_query": known,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        // The tracing::warn above is invisible at the default EnvFilter
        // level, so the TTY ack itself must carry the orphan notice.
        if known {
            writeln!(out, "ok")?;
        } else {
            writeln!(
                out,
                "ok (query id not in log — evicted or never logged; recorded anyway)"
            )?;
        }
    }
    Ok(())
}
