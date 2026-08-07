//! `api::tune::{Request, Response, run}` — the shared middle of
//! `comemory tune` / `POST /api/v1/tune`: search the `[tune]` blend-knob
//! grid against the merged golden set (tracking off) and, when `apply` is
//! set and the winner strictly beats the current config, persist it into
//! `config.toml`. Moved out of `cli::tune::run` (Binding Rule 1).
//!
//! `tune` is classified `mutating` regardless of `apply` (§Route map
//! Notes) — the route confirm-gates the request only when `req.apply` is
//! true, but the write permit + read-only gate always apply, since a
//! report-only run still burns real CPU time that a `--read-only` server
//! should refuse alongside every other mutating job.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::api::eval::default_k;
use crate::eval::golden;
use crate::eval::tune::{self, TuneReport};
use crate::prelude::*;

/// `comemory tune` / `POST /api/v1/tune` request.
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
    /// Rewrite `config.toml` with the winning knobs when (and only when)
    /// the winner strictly beats the current config.
    #[serde(default)]
    pub apply: bool,
    /// Seed for the candidate sampler (`[tune] samples > 0`). Omitted, the
    /// seed is derived from the golden-set size, grid shape, and schema
    /// version — so runs are reproducible either way.
    #[serde(default)]
    pub seed: Option<u64>,
}

/// `comemory tune`'s existing `--json` shape (`{"report":..,"applied":..}`),
/// promoted to a typed `Response` so the CLI and the HTTP handler share one
/// type instead of each hand-building the same anonymous object.
#[derive(Serialize, Debug)]
pub struct Response {
    /// The full ranked-candidate report.
    pub report: TuneReport,
    /// Whether `config.toml` was rewritten.
    pub applied: bool,
}

/// Build the merged golden set, search the blend knobs through the real
/// pipeline (tracking off), and — when `req.apply` and the winner strictly
/// beats the baseline — persist the winner into `config.toml`.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let cfg = ctx.cfg;
    let paths = ctx.paths;
    let conn = ctx.conn()?;
    let pairs = golden::resolve(
        &*conn,
        req.golden.as_deref().map(Path::new),
        req.golden_only,
    )?;
    let min_pairs = tune::resolve_min_pairs()?;
    let report = tune::run_tune(cfg, &*conn, &pairs, req.k, min_pairs, req.seed)?;
    let winner = report.winner()?;

    let applied = req.apply && report.improves_baseline();
    if applied {
        tune::apply_to_config_file(&paths.config_file(), &winner.candidate)?;
    }

    Ok(Response { report, applied })
}

#[cfg(test)]
#[path = "tests/tune.rs"]
mod tests;
