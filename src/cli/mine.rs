//! `comemory mine` — distill (failed_term → fix_term) expansion mappings
//! from reformulation pairs in `retrieval_log`, and optionally rebuild
//! the `query_expansions` table the tier-4 lexical ladder reads.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use time::OffsetDateTime;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::eval::mine;
use crate::output::json;
use crate::prelude::*;
use crate::store::{connection, memory_row};

const EXAMPLES: &str = "\
Examples:
  # Report mined expansion mappings without changing retrieval
  comemory mine

  # Rebuild the query_expansions table from the current retrieval_log
  comemory mine --apply

  # Machine-readable report
  comemory mine --json";

/// Arguments to `comemory mine`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Rebuild the query_expansions table from the mined mappings
    /// (default: report only).
    #[arg(long, default_value_t = false)]
    pub apply: bool,
}

/// Run `comemory mine`: scan `retrieval_log` for (failed → used-feedback)
/// reformulation pairs, report the distilled term mappings, and rebuild
/// `query_expansions` from them when `--apply` is set.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;

    let mappings = mine::mine(&conn)?;
    if a.apply {
        let now_iso = memory_row::iso_format(OffsetDateTime::now_utc())?;
        mine::apply(&mut conn, &mappings, &now_iso)?;
    }
    if json_flag {
        json::write(&serde_json::json!({
            "mappings": mappings,
            "applied": a.apply,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        for m in &mappings {
            writeln!(out, "{} -> {} (support {})", m.term, m.expansion, m.support)?;
        }
        if a.apply {
            writeln!(out, "(applied)")?;
        } else {
            writeln!(out, "(report only — use --apply)")?;
        }
    }
    Ok(())
}
