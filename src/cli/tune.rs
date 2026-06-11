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

use crate::cli::{load_config, resolve_data_dir};
use crate::config::paths::Paths;
use crate::eval::golden;
use crate::eval::tune::{self, ScoredCandidate, TuneCandidate};
use crate::output::json;
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Grid-search 81 configs against the merged golden set (report only)
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
    /// Rewrite config.toml with the winning knobs when (and only when)
    /// the winner strictly beats the current config. Comments in an
    /// existing config.toml are dropped by the rewrite.
    #[arg(long, default_value_t = false)]
    pub apply: bool,
}

/// Resolve the minimum-golden-pairs floor: `COMEMORY_TUNE_MIN_GOLDEN`
/// when set (a test hook — invalid values are a hard error naming the
/// variable), else [`tune::MIN_GOLDEN_PAIRS`].
fn min_golden_pairs() -> Result<usize> {
    match std::env::var("COMEMORY_TUNE_MIN_GOLDEN") {
        Ok(raw) => raw.trim().parse::<usize>().map_err(|_| {
            Error::Other(format!(
                "COMEMORY_TUNE_MIN_GOLDEN: expected a non-negative integer, got {raw:?}"
            ))
        }),
        Err(_) => Ok(tune::MIN_GOLDEN_PAIRS),
    }
}

/// True when the winner strictly beats the baseline on mrr, with
/// recall@k breaking exact mrr ties.
fn strictly_beats(winner: &ScoredCandidate, baseline: &ScoredCandidate) -> bool {
    match winner.mrr.total_cmp(&baseline.mrr) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => winner.recall_at_k > baseline.recall_at_k,
        std::cmp::Ordering::Less => false,
    }
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

    let pairs = golden::resolve(&conn, a.golden.as_deref(), a.golden_only)?;
    let min_pairs = min_golden_pairs()?;
    let report = tune::run_tune(&cfg, &conn, &pairs, a.k, min_pairs)?;
    let winner = report
        .ranked
        .first()
        .ok_or_else(|| Error::Other("tune produced an empty candidate ranking".into()))?;

    let improved = strictly_beats(winner, &report.baseline);
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
