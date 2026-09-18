//! `comemory benchmark` — score a reviewed, versioned benchmark set over the
//! real retrieval legs and emit a replayable artifact.
//!
//! CLI-only: the run writes its artifact to an operator-named filesystem path,
//! which is what already keeps `install` and `setup` off the HTTP surface.

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;

use crate::cli::load_config;
use crate::cli::output::json;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::learning::benchmark;
use crate::domains::learning::evaluation::benchmark_arm_report::ArmReport;
use crate::domains::learning::evaluation::benchmark_report::BenchmarkReport;
use crate::prelude::*;
use crate::store::connection;
use crate::utilities::context::Ctx;

const EXAMPLES: &str = "\
Examples:
  # Score the deterministic baseline over a reviewed set
  comemory benchmark --set benchmark.yaml

  # Write the replayable artifact, then score an external scorer over it
  comemory benchmark --set benchmark.yaml --report run.json
  comemory benchmark --set benchmark.yaml --scores reranker.json --json

  # Compare two scorers against one candidate snapshot at recall@10
  comemory benchmark --set benchmark.yaml --scores base.json --scores lora.json --k 10";

/// Arguments to `comemory benchmark`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Path to the reviewed benchmark set YAML.
    #[arg(long)]
    pub set: PathBuf,
    /// Write the full replayable artifact JSON here (created or truncated).
    #[arg(long)]
    pub report: Option<PathBuf>,
    /// Add one arm from a scorer's JSON scores file. Repeatable.
    #[arg(long)]
    pub scores: Vec<PathBuf>,
    /// Override the set's recall@k / nDCG@k cut.
    #[arg(long, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    pub k: Option<usize>,
}

/// Run `comemory benchmark`: capture every task's candidate pool, score every
/// arm over that one snapshot, print the summary and optionally write the
/// artifact.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let report = benchmark::run(
        &mut ctx,
        &benchmark::Request {
            set: a.set,
            scores: a.scores,
            k: a.k,
        },
    )?;

    if json_flag {
        json::write(&report.summary())?;
    } else {
        write_tty(&report)?;
    }
    // The artifact is written LAST, so a bad path cannot lose a measured run:
    // the summary has already reached the caller by the time this can fail.
    match a.report {
        Some(path) => write_report(&path, &report),
        None => Ok(()),
    }
}

/// Serialize the whole artifact, candidates included, to `path`.
fn write_report(path: &Path, report: &BenchmarkReport) -> Result<()> {
    let body = serde_json::to_vec_pretty(report).map_err(Error::Json)?;
    std::fs::write(path, body).map_err(Error::Io)
}

/// The human summary: the dataset, what it was measured against, then one
/// block per arm with its per-domain breakdown.
fn write_tty(report: &BenchmarkReport) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} v{}: {} tasks, {} judgments, k={}",
        report.set_name,
        report.set_size.version,
        report.set_size.tasks,
        report.set_size.judgments,
        report.k
    )?;
    writeln!(
        out,
        "corpus {} ({} memories, {} symbols, {} documents)  knobs {}  decay {}",
        short(&report.retrieval.corpus.digest),
        report.retrieval.corpus.memories,
        report.retrieval.corpus.code_symbols,
        report.retrieval.corpus.documents,
        short(&report.retrieval.knobs_hash),
        if report.decay_frozen {
            "frozen"
        } else {
            "live (results depend on reference_time)"
        }
    )?;
    for arm in &report.arms {
        write_arm(&mut out, arm)?;
    }
    Ok(())
}

/// One arm's block: its aggregate, its paired delta, and its per-domain rows.
fn write_arm(out: &mut impl Write, arm: &ArmReport) -> Result<()> {
    let m = &arm.overall;
    writeln!(
        out,
        "\n{} [{:?}]  pool_recall {:.3}  recall@k {:.3}  mrr {:.3}  ndcg@k {:.3}  \
         ({}/{} judged tasks, {:.0}% of pages judged)",
        arm.name,
        arm.verdict,
        m.pool_recall,
        m.recall_at_k,
        m.mrr,
        m.ndcg_at_k,
        m.judged_tasks,
        m.tasks,
        m.judged_page_fraction * 100.0
    )?;
    writeln!(
        out,
        "  latency p50 {}ms p95 {}ms max {}ms ({})",
        arm.latency_ms.p50,
        arm.latency_ms.p95,
        arm.latency_ms.max,
        if arm.latency_within_budget {
            "within budget"
        } else {
            "OVER BUDGET"
        }
    )?;
    if let Some(delta) = &arm.paired_vs_baseline {
        writeln!(
            out,
            "  paired {} vs baseline: {:+.4} [{:+.4}, {:+.4}] over {} tasks",
            delta.metric, delta.mean_delta, delta.ci.0, delta.ci.1, delta.tasks
        )?;
    }
    for (domain, summary) in &arm.per_domain {
        if summary.judged_tasks == 0 {
            continue;
        }
        writeln!(
            out,
            "  {domain:<9} pool_recall {:.3}  recall@k {:.3}  ndcg@k {:.3}  ({} tasks)",
            summary.pool_recall, summary.recall_at_k, summary.ndcg_at_k, summary.judged_tasks
        )?;
    }
    Ok(())
}

/// The first 12 characters of a digest, enough to compare two runs by eye.
fn short(digest: &str) -> &str {
    digest.get(..12).unwrap_or(digest)
}
