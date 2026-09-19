//! `comemory recall-status` — tracked queries, verdicts, saves, and the
//! still-pending recalls for a repo + lower time bound. The shared middle is
//! `domains::learning::recall_status` (Binding Rule 1), shared verbatim with
//! `GET /api/v1/learning/recall-status` and the MCP `recall_status` tool.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::load_config;
use crate::cli::output::json;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::learning::recall_status;
use crate::prelude::*;
use crate::utilities::context::Ctx;

const EXAMPLES: &str = "\
Examples:
  # Everything tracked since the start of today
  comemory recall-status

  # Scope to one repo
  comemory recall-status --repo comemory

  # A wider window
  comemory recall-status --since 2026-09-01

  # JSON for a dashboard or an agent's own recall loop
  comemory recall-status --repo comemory --json";

/// Arguments to `comemory recall-status`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Restrict every count to this repo. Unset reports across every repo.
    #[arg(long)]
    pub repo: Option<String>,
    /// Lower time bound. Accepts an RFC3339 timestamp or a bare
    /// `YYYY-MM-DD` date (start of that UTC day) — the same grammar
    /// `search --since` accepts. Defaults to the start of the current UTC
    /// day when omitted.
    #[arg(long, value_name = "WHEN")]
    pub since: Option<String>,
}

/// Report tracked queries, verdicts, saves, and pending recalls via
/// `domains::learning::recall_status::run`, which guards on `comemory.db`
/// existing (like `domains::maintenance::stats::run`): a data dir with no
/// database yet reports zeros rather than creating one. Uses a lazy `Ctx`
/// so that guard can run before any connection is opened.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let output = recall_status::run(
        &mut ctx,
        recall_status::Request {
            repo: a.repo,
            since: a.since,
        },
    )?;
    emit(json_flag, &output)
}

/// Emit the report: the core `Output` verbatim under `--json`, else a header
/// line with the four counters followed by one line per still-pending
/// tracked query (`query_id`, `source`, `query`, `returned_ids`).
fn emit(json_flag: bool, output: &recall_status::Output) -> Result<()> {
    if json_flag {
        return json::write(output);
    }
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "repo={}  since={}  queries={}  feedback_events={}  saves={}  pending={}",
        output.repo.as_deref().unwrap_or("*"),
        output.since,
        output.queries,
        output.feedback_events,
        output.saves,
        output.pending.len(),
    )?;
    for row in &output.pending {
        writeln!(
            out,
            "  {}  {:<10}  {}  [{}]",
            row.query_id,
            row.source,
            row.query,
            row.returned_ids.join(","),
        )?;
    }
    Ok(())
}
