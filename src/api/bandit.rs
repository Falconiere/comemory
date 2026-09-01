//! `api::bandit::{Request, run}` — the shared middle of `comemory bandit`
//! / `POST /api/v1/bandit`: Thompson-sample the `[tune]` grid, confirm the
//! sample against the golden set, and — with `apply` — persist a winning
//! arm into `config.toml`. Moved out of `cli::bandit::run` (Binding Rule 1).
//!
//! **Always mutating, regardless of `apply`** (§Route map Notes):
//! [`crate::eval::bandit::run_bandit`] unconditionally seeds/updates
//! `bandit_arms` (`seed_arms` + `record_outcome`), so even a report-only
//! run (`apply: false`) writes to the DB. The HTTP route confirm-gates the
//! request only when `req.apply` is true, but the write permit + read-only
//! gate always apply.

use rusqlite::Connection;
use serde::Deserialize;

use crate::api::Ctx;
use crate::api::eval::{RunOutcome, default_k, record_run};
use crate::eval::bandit::{self, BanditReport};
use crate::eval::golden;
use crate::eval::runner;
use crate::eval::tune;
use crate::prelude::*;

/// `comemory bandit` / `POST /api/v1/bandit` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Path to a YAML golden file. Over HTTP, containment is enforced by
    /// the route handler BEFORE this runs (§Security "Path containment").
    #[serde(default)]
    pub golden: Option<String>,
    /// Skip the feedback harvest; use only `golden`.
    #[serde(default)]
    pub golden_only: bool,
    /// recall@k cut.
    #[serde(default = "default_k")]
    pub k: usize,
    /// Rewrite `config.toml` when the sampled arm strictly beats baseline.
    #[serde(default)]
    pub apply: bool,
}

/// Seed the arm table, Thompson-sample one arm, confirm it against the
/// golden set, and — with `req.apply` and a win — persist it into
/// `config.toml`. Refuses `req.apply` up front when `[bandit] enabled =
/// false` in the effective config (same pre-flight refusal as the CLI).
/// Records this run (the confirmed arm's knobs + score, `applied`) in
/// `eval_runs` via [`record_run`] (`kind = "bandit"`).
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<BanditReport> {
    let cfg = ctx.cfg;
    if req.apply && !cfg.bandit.enabled {
        return Err(Error::Config(
            "bandit --apply refused: [bandit] enabled = false in config.toml".into(),
        ));
    }
    let paths = ctx.paths;
    let conn: &mut Connection = ctx.conn()?;
    let pairs = golden::resolve(
        &*conn,
        req.golden.as_deref().map(std::path::Path::new),
        req.golden_only,
    )?;
    let min_pairs = tune::resolve_min_pairs()?;
    let report = bandit::run_bandit(
        cfg,
        conn,
        &pairs,
        req.k,
        min_pairs,
        req.apply,
        &paths.config_file(),
    )?;

    // `BanditReport` carries the confirmed candidate's knobs and `applied`
    // but not its recall/MRR (only `baseline_*` and each arm's `last_mrr`
    // are persisted); replay it once more (same pipeline, same golden
    // pairs, tracking off) to get the real pair for this run's `eval_runs`
    // row — deterministic, so it reproduces exactly what `run_bandit`
    // already scored internally.
    let confirmed = runner::run_eval(
        &tune::with_candidate(cfg, &report.proposed),
        &*conn,
        &pairs,
        req.k,
    )?;
    let knobs_json = serde_json::to_string(&report.proposed).map_err(Error::Json)?;
    record_run(
        conn,
        RunOutcome {
            kind: "bandit",
            golden_pairs: report.golden_pairs,
            k: report.k,
            recall: confirmed.recall_at_k,
            mrr: confirmed.mrr,
            knobs_json,
            applied: report.applied,
        },
    )?;

    Ok(report)
}

#[cfg(test)]
#[path = "tests/bandit.rs"]
mod tests;
