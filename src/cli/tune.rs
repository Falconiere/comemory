//! `comemory tune` — deterministic search over the six blend knobs
//! (rrf_k, decay, mmr_lambda, bm25_weights, graph_hops, graph_seeds),
//! scored by eval MRR with recall@k as the tie-break, and an opt-in
//! `--apply` that writes the winner into `config.toml`. `[tune] samples`
//! decides whether a run enumerates the grid or samples it; `--seed`
//! pins the draw.
//!
//! `COMEMORY_TUNE_MIN_GOLDEN` overrides the minimum-golden-pairs floor.
//! It is a test hook (documented as such), not a tuning knob.

use std::io::Write;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::eval::GoldenSetArgs;
use crate::cli::{load_config, resolve_data_dir};
use crate::config::paths::Paths;
use crate::eval::golden;
use crate::eval::tune::{self, TuneCandidate};
use crate::output::json;
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Search the configured [tune] surface (64 sampled configs out of 729
  # by default) against the merged golden set (report only)
  comemory tune

  # Reproduce an exact candidate draw ([tune] samples > 0); without
  # --seed the seed is derived from the golden size and grid shape
  comemory tune --seed 42

  # File-only golden set, recall@5, machine-readable report
  # (JSON envelope: {\"report\": <TuneReport>, \"applied\": bool})
  comemory tune --golden golden.yaml --golden-only --k 5 --json

  # Write the winning knobs into config.toml (atomic rename; the file
  # is re-rendered from parsed TOML, so comments are dropped)
  comemory tune --golden golden.yaml --apply";

/// Arguments to `comemory tune`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Golden-set selection (`--golden`, `--golden-only`, `--k`),
    /// shared with `comemory eval`.
    #[command(flatten)]
    pub golden_set: GoldenSetArgs,
    /// Rewrite config.toml with the winning knobs when (and only when)
    /// the winner strictly beats the current config. Comments in an
    /// existing config.toml are dropped by the rewrite.
    #[arg(long, default_value_t = false)]
    pub apply: bool,
    /// Seed for the candidate sampler (`[tune] samples > 0`). Omitted, the
    /// seed is derived from the golden-set size, grid shape, and schema
    /// version — so runs are reproducible either way. No effect when
    /// `samples = 0` (exhaustive grid).
    #[arg(long)]
    pub seed: Option<u64>,
}

/// Render one candidate's knobs for the TTY view.
fn fmt_candidate(c: &TuneCandidate) -> String {
    format!(
        "rrf_k={} decay={} mmr_lambda={} bm25=({},{}) graph_hops={} graph_seeds={}",
        c.rrf_k,
        c.decay,
        c.mmr_lambda,
        c.bm25_weights.0,
        c.bm25_weights.1,
        c.graph_hops,
        c.graph_seeds
    )
}

/// TTY view: baseline, winner delta, and the top 5 of the ranking.
fn render(out: &mut impl Write, report: &tune::TuneReport) -> Result<()> {
    let b = &report.baseline;
    let w = report.winner()?;
    writeln!(
        out,
        "baseline: mrr {:.3} recall@{} {:.3}  ({})",
        b.mrr,
        report.k,
        b.recall_at_k,
        fmt_candidate(&b.candidate)
    )?;
    writeln!(
        out,
        "winner:   mrr {:.3} -> {:.3} ({:+.3})  recall@{} {:.3} -> {:.3} ({:+.3})",
        b.mrr,
        w.mrr,
        w.mrr - b.mrr,
        report.k,
        b.recall_at_k,
        w.recall_at_k,
        w.recall_at_k - b.recall_at_k
    )?;
    writeln!(out, "          {}", fmt_candidate(&w.candidate))?;
    writeln!(out, "top 5 of {} candidates:", report.ranked.len())?;
    for (i, s) in report.ranked.iter().take(5).enumerate() {
        writeln!(
            out,
            "  {}. mrr {:.3} recall {:.3}  {}",
            i + 1,
            s.mrr,
            s.recall_at_k,
            fmt_candidate(&s.candidate)
        )?;
    }
    Ok(())
}

/// Run `comemory tune`: build the merged golden set, search the blend
/// knobs through the real pipeline (tracking off), and report — or, with
/// `--apply`, persist a strictly-better winner to config.toml.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;

    let g = &a.golden_set;
    let pairs = golden::resolve(&conn, g.golden.as_deref(), g.golden_only)?;
    let min_pairs = tune::resolve_min_pairs()?;
    let report = tune::run_tune(&cfg, &conn, &pairs, g.k, min_pairs, a.seed)?;
    let winner = report.winner()?;

    let improved = report.improves_baseline();
    let applied = a.apply && improved;
    if applied {
        tune::apply_to_config_file(&paths.config_file(), &winner.candidate)?;
    }

    if json_flag {
        json::write(&serde_json::json!({
            "report": report,
            "applied": applied,
        }))?;
        return Ok(());
    }

    let mut out = std::io::stdout().lock();
    render(&mut out, &report)?;
    if applied {
        writeln!(out, "(applied to {})", paths.config_file().display())?;
    } else if !improved {
        writeln!(out, "current config already optimal; nothing applied")?;
    } else {
        writeln!(out, "(report only — re-run with --apply)")?;
    }
    Ok(())
}
