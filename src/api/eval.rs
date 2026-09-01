//! `api::eval::{Request, run}` — the shared middle of `comemory eval` /
//! `POST /api/v1/eval`: build the merged golden set (file ∪ feedback
//! harvest) and score the real pipeline against it with tracking off.
//! Moved out of `cli::eval::run` (Binding Rule 1). [`history`] is the
//! shared middle of `comemory eval --history` / `GET /api/v1/eval/history`.
//! Every run of either kind — and of `api::tune::run` / `api::bandit::run`
//! — is recorded via [`record_run`] into `eval_runs`, one row per run.
//!
//! `eval` mutates nothing (§Route map Notes): the run is read-class even
//! though it can take a while, which is why the HTTP route runs it as a
//! non-mutating job (no write permit, unaffected by `--read-only`).
//! `eval --history` is a plain `SELECT`, so its route runs synchronously
//! instead.

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::api::Ctx;
use crate::eval::golden;
use crate::eval::runner::{self, EvalReport};
use crate::prelude::*;
use crate::store::{eval_runs, memory_row, random_id};

/// `comemory eval` / `POST /api/v1/eval` request — also, via its `history`/
/// `limit` fields, `comemory eval --history` / `GET /api/v1/eval/history`'s
/// request. One `Request` type rather than two: `tests/api__parity.rs`
/// (AC-41) probes exactly one `api::<cmd>::Request` per clap subcommand, and
/// `eval --history` is a second mode of the same `eval` subcommand, not a
/// separate one. [`run`] never reads `history`/`limit`; [`history`] never
/// reads `golden`/`golden_only`/`k` — the caller (CLI arg-branch, or the
/// HTTP route: `POST /eval` vs `GET /eval/history`) picks which function
/// runs, so the two field groups never interact within one call.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Path to a YAML golden file (`- query: ...` / `  relevant: [..]`).
    /// Over HTTP, filesystem containment is enforced by the route handler
    /// BEFORE this runs (§Security "Path containment") — `run` treats it
    /// as an already-safe path.
    #[serde(default)]
    pub golden: Option<String>,
    /// Skip the feedback harvest; use only `golden`.
    #[serde(default)]
    pub golden_only: bool,
    /// recall@k cut.
    #[serde(default = "default_k")]
    pub k: usize,
    /// Read past `eval`/`tune`/`bandit` runs instead of scoring a new one.
    /// Read only by the CLI's own mode branch — `run` and `history` each
    /// ignore it.
    #[serde(default)]
    pub history: bool,
    /// Max rows [`history`] returns, newest-first.
    #[serde(default = "default_history_limit")]
    pub limit: u32,
}

/// The default `limit` for `eval --history` / `GET /api/v1/eval/history`.
pub(crate) fn default_history_limit() -> u32 {
    20
}

/// The CLI's `--k` default (`GoldenSetArgs`), reused by `tune`/`bandit`'s
/// `Request` too so an HTTP request omitting `k` scores identically to the
/// CLI across all three commands.
pub(crate) fn default_k() -> usize {
    3
}

/// Build the merged golden set and score the real pipeline against it
/// (tracking off — measurement must not feed the signals it measures),
/// then record this run in `eval_runs` (`kind = "eval"`).
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<EvalReport> {
    let cfg = ctx.cfg;
    let conn = ctx.conn()?;
    let pairs = golden::resolve(
        &*conn,
        req.golden.as_deref().map(Path::new),
        req.golden_only,
    )?;
    let report = runner::run_eval(cfg, &*conn, &pairs, req.k)?;
    record_run(
        conn,
        RunOutcome {
            kind: "eval",
            golden_pairs: report.queries,
            k: report.k,
            recall: report.recall_at_k,
            mrr: report.mrr,
            // `eval` scores no candidate grid — the "knobs" scored are the
            // live config's six blend knobs, in the same shape
            // `tune`/`bandit` snapshot for their winner, so the console's
            // per-row knob display is uniform across all three `kind`s.
            knobs_json: current_knobs_json(cfg)?,
            applied: false,
        },
    )?;
    Ok(report)
}

/// Read back up to `req.limit` past `eval`/`tune`/`bandit` runs, newest-first
/// (`req.golden`/`golden_only`/`k`/`history` are ignored — see the module
/// doc on [`Request`]).
pub fn history(ctx: &mut Ctx<'_>, req: &Request) -> Result<Vec<eval_runs::EvalRunRow>> {
    let conn = ctx.conn()?;
    eval_runs::list(&*conn, req.limit)
}

/// Random bytes behind an `eval_runs` row id — 8 bytes, rendered as 16
/// lowercase-hex chars (the same width [`crate::api::gc`] uses for
/// `gc_runs`).
const RUN_ID_BYTES: usize = 8;

/// One completed `eval`/`tune`/`bandit` run, ready to record. Bundled into
/// a struct rather than positional arguments (`clippy::too_many_arguments`).
pub(crate) struct RunOutcome<'a> {
    /// `"eval"` | `"tune"` | `"bandit"`.
    pub kind: &'a str,
    /// Golden pairs scored.
    pub golden_pairs: usize,
    /// recall@k cut used.
    pub k: usize,
    /// Mean recall@k for this run's reported candidate.
    pub recall: f64,
    /// Mean MRR for this run's reported candidate.
    pub mrr: f64,
    /// Pre-serialized JSON text of the scored knob set.
    pub knobs_json: String,
    /// Whether this run rewrote `config.toml`.
    pub applied: bool,
}

/// Insert one `eval_runs` row for a completed run — shared by `eval`
/// (here) and by `api::tune::run` / `api::bandit::run`, so id generation,
/// the timestamp, and the row shape live in one place (Binding Rule 1).
pub(crate) fn record_run(conn: &Connection, outcome: RunOutcome<'_>) -> Result<()> {
    let id = random_id::random_hex(RUN_ID_BYTES)?;
    let at = memory_row::iso_format(OffsetDateTime::now_utc())?;
    eval_runs::insert(
        conn,
        &eval_runs::NewRun {
            id: &id,
            kind: outcome.kind,
            at: &at,
            golden_pairs: outcome.golden_pairs as u64,
            k: outcome.k as u64,
            recall: outcome.recall,
            mrr: outcome.mrr,
            knobs: &outcome.knobs_json,
            applied: outcome.applied,
        },
    )
}

/// Serialize the live config's six blend knobs into the same field shape
/// [`crate::eval::tune::TuneCandidate`] carries, without depending on that
/// type (`eval::tune` is not this step's — F16 — surface).
fn current_knobs_json(cfg: &crate::config::Config) -> Result<String> {
    #[derive(Serialize)]
    struct LiveKnobs {
        rrf_k: f32,
        decay: f64,
        mmr_lambda: f64,
        bm25_weights: (f32, f32),
        graph_hops: u32,
        graph_seeds: usize,
    }
    serde_json::to_string(&LiveKnobs {
        rrf_k: cfg.retrieval.rrf_k,
        decay: cfg.rank.decay,
        mmr_lambda: cfg.rank.mmr_lambda,
        bm25_weights: cfg.retrieval.bm25_weights,
        graph_hops: cfg.retrieval.graph_hops,
        graph_seeds: cfg.retrieval.graph_seeds,
    })
    .map_err(Error::Json)
}

#[cfg(test)]
#[path = "tests/eval.rs"]
mod tests;
