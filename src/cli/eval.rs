//! `comemory eval` — score retrieval quality (recall@k, MRR) against a
//! golden set harvested from feedback and/or loaded from a YAML file.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Score retrieval against feedback-harvested golden pairs
  comemory eval

  # Merge a hand-written golden file (file wins on duplicate query)
  comemory eval --golden golden.yaml

  # File only, recall@5, JSON report
  comemory eval --golden golden.yaml --golden-only --k 5 --json";

/// Golden-set selection flags shared by `comemory eval` and
/// `comemory tune` (via `#[command(flatten)]`), so the two subcommands
/// cannot drift on file/harvest/k semantics or help text.
#[derive(ClapArgs, Debug)]
pub struct GoldenSetArgs {
    /// Path to a YAML golden file (`- query: ...` / `  relevant: [..]`).
    #[arg(long)]
    pub golden: Option<PathBuf>,
    /// Skip the feedback harvest; use only the --golden file.
    #[arg(long, default_value_t = false, requires = "golden")]
    pub golden_only: bool,
    /// recall@k cut (defaults to 3).
    #[arg(long, default_value_t = 3,
          value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    pub k: usize,
}

/// Arguments to `comemory eval`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Golden-set selection (`--golden`, `--golden-only`, `--k`).
    #[command(flatten)]
    pub golden_set: GoldenSetArgs,
}

/// Run `comemory eval`: build the merged golden set, drive the real
/// pipeline with tracking off, and emit the report.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;

    let g = &a.golden_set;
    let req = api::eval::Request {
        golden: g.golden.as_ref().map(|p| p.to_string_lossy().into_owned()),
        golden_only: g.golden_only,
        k: g.k,
    };
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let report = api::eval::run(&mut ctx, req)?;
    if json_flag {
        json::write(&report)?;
    } else {
        let mut out = std::io::stdout().lock();
        // Brackets are the 95% bootstrap intervals: a gap between two runs
        // that sits inside them is noise, not a ranking change.
        writeln!(
            out,
            "recall@{}: {:.3} [{:.3}, {:.3}]  mrr: {:.3} [{:.3}, {:.3}]  ({} queries)",
            report.k,
            report.recall_at_k,
            report.recall_ci.0,
            report.recall_ci.1,
            report.mrr,
            report.mrr_ci.0,
            report.mrr_ci.1,
            report.queries
        )?;
        for r in report.results.iter().take(5) {
            writeln!(
                out,
                "  worst: {:?} recall={:.2} first_hit={}",
                r.query,
                r.recall,
                r.rank_of_first_hit.map_or("miss".into(), |x| x.to_string())
            )?;
        }
    }
    Ok(())
}
