//! `comemory bandit` — Thompson sample over the `[tune]` grid, confirm with
//! offline eval, optionally `--apply` when the sample beats baseline.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::eval::GoldenSetArgs;
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::eval::bandit::BanditReport;
use crate::eval::tune::TuneCandidate;
use crate::output::json;
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Thompson-sample one arm, confirm vs baseline (report only)
  comemory bandit

  # Write knobs into config.toml when the sample beats baseline
  comemory bandit --golden golden.yaml --apply --json";

/// Arguments to `comemory bandit`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Golden-set selection (`--golden`, `--golden-only`, `--k`).
    #[command(flatten)]
    pub golden_set: GoldenSetArgs,
    /// Rewrite config.toml when the sampled arm strictly beats baseline.
    #[arg(long, default_value_t = false)]
    pub apply: bool,
}

fn fmt_candidate(c: &TuneCandidate) -> String {
    format!(
        "rrf_k={} decay={} mmr_lambda={} bm25=({},{})",
        c.rrf_k, c.decay, c.mmr_lambda, c.bm25_weights.0, c.bm25_weights.1
    )
}

/// TTY view: baseline, proposed arm, and the top posterior mean.
fn render(out: &mut impl std::io::Write, report: &BanditReport) -> Result<()> {
    writeln!(
        out,
        "baseline: mrr {:.3} recall@{} {:.3}",
        report.baseline_mrr, report.k, report.baseline_recall_at_k
    )?;
    writeln!(
        out,
        "proposed: {}  applied={}",
        fmt_candidate(&report.proposed),
        report.applied
    )?;
    if let Some(top) = report.ranked.first() {
        writeln!(
            out,
            "top mean: {:.3}  {}  pulls={}",
            top.mean(),
            fmt_candidate(&top.candidate),
            top.pulls
        )?;
    }
    Ok(())
}

/// Run `comemory bandit`: build the merged golden set, Thompson-sample and
/// confirm one arm through the real pipeline (tracking off), and — with
/// `--apply` — persist a winning arm into `config.toml`.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;

    let g = &a.golden_set;
    let req = api::bandit::Request {
        golden: g.golden.as_ref().map(|p| p.to_string_lossy().into_owned()),
        golden_only: g.golden_only,
        k: g.k,
        apply: a.apply,
    };
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let report = api::bandit::run(&mut ctx, req)?;

    if json_flag {
        json::write(&serde_json::json!({ "report": report }))?;
        return Ok(());
    }

    let mut out = std::io::stdout().lock();
    render(&mut out, &report)?;
    Ok(())
}
