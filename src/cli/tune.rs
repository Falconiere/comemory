//! `comemory tune` — deterministic grid search over the four blend
//! knobs (rrf_k, decay, mmr_lambda, bm25_weights), scored by eval MRR
//! with recall@k as the tie-break, and an opt-in `--apply` that writes
//! the winner into `config.toml`.
//!
//! `COMEMORY_TUNE_MIN_GOLDEN` overrides the minimum-golden-pairs floor.
//! It is a test hook (documented as such), not a tuning knob.

use std::io::Write as _;
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
  # Grid-search the configured [tune] grid (81 configs by default)
  # against the merged golden set (report only)
  comemory tune

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
}

/// Render one candidate's knobs for the TTY view.
fn fmt_candidate(c: &TuneCandidate) -> String {
    format!(
        "rrf_k={} decay={} mmr_lambda={} bm25=({},{})",
        c.rrf_k, c.decay, c.mmr_lambda, c.bm25_weights.0, c.bm25_weights.1
    )
}

/// Run `comemory tune`: build the merged golden set, grid-search the
/// blend knobs through the real pipeline (tracking off), and report —
/// or, with `--apply`, persist a strictly-better winner to config.toml.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;

    let g = &a.golden_set;
    let pairs = golden::resolve(&conn, g.golden.as_deref(), g.golden_only)?;
    let min_pairs = tune::resolve_min_pairs()?;
    let report = tune::run_tune(&cfg, &conn, &pairs, g.k, min_pairs)?;
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
    let b = &report.baseline;
    let w = winner;
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
    if applied {
        writeln!(out, "(applied to {})", paths.config_file().display())?;
    } else if !improved {
        writeln!(out, "current config already optimal; nothing applied")?;
    } else {
        writeln!(out, "(report only — re-run with --apply)")?;
    }
    Ok(())
}
