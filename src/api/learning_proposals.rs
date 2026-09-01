//! `api::learning_proposals` — knob proposals derived from unapplied
//! `tune`/`bandit` runs: list, apply, discard (console-api spec §7).
//!
//! A *proposal* is not a stored row of its own: it is a `tune`/`bandit`
//! run whose recorded winning knobs (`eval_runs.knobs`) still differ from
//! the live config, and which has been neither applied nor discarded. That
//! derivation is why v15 added `eval_runs.discarded` — "applied" was
//! already recorded, and "differs from live config" is computable, but
//! "dismissed without applying" was not expressible.
//!
//! [`apply`] writes through the same `eval::tune::apply_to_config_file`
//! `comemory tune --apply` uses (Binding Rule 1: one config writer), then
//! stamps the row `applied` so the proposal stops being offered.

use serde::Serialize;
use serde_json::Value;

use crate::api::Ctx;
use crate::config::Config;
use crate::eval::tune::{self, TuneCandidate};
use crate::prelude::*;
use crate::store::eval_runs::{self, EvalRunRow};

/// Read the whole run history: a proposal can be arbitrarily old, and the
/// list is filtered down to unapplied/undiscarded rows anyway.
const ALL_RUNS: u32 = u32::MAX;

/// One knob whose proposed value differs from the live one.
#[derive(Serialize, Debug)]
pub struct KnobChange {
    /// Knob name, in `config.toml` spelling (`rrf_k`, `mmr_lambda`, …).
    pub name: &'static str,
    /// The live config's current value.
    pub from: Value,
    /// The value this run's winner would set.
    pub to: Value,
}

/// One offered proposal: the run that produced it plus the per-knob diff.
#[derive(Serialize, Debug)]
pub struct Proposal {
    /// `eval_runs` row id — the `{id}` of the apply/discard routes.
    pub id: String,
    /// `"tune"` | `"bandit"` (an `eval` run proposes nothing).
    pub kind: String,
    /// ISO-8601 UTC timestamp the run completed.
    pub at: String,
    /// Mean recall@k the proposed knobs scored.
    pub recall_at_k: f64,
    /// Mean MRR the proposed knobs scored.
    pub mrr: f64,
    /// Golden pairs the run scored against.
    pub golden_pairs: u64,
    /// recall@k cut used.
    pub k: u64,
    /// Every knob that would change, live value and proposed value. Never
    /// empty — a run matching the live config is not offered at all.
    pub knobs: Vec<KnobChange>,
}

/// `POST /api/v1/learning/proposals/{id}/apply`'s response.
#[derive(Serialize, Debug)]
pub struct ApplyResponse {
    /// The applied run's id.
    pub id: String,
    /// Always `true` — a failed apply is an error, not a `false` here.
    pub applied: bool,
    /// The `config.toml` that was rewritten, so a client can say where.
    pub config_file: String,
}

/// `POST /api/v1/learning/proposals/{id}/discard`'s response.
#[derive(Serialize, Debug)]
pub struct DiscardResponse {
    /// The dismissed run's id.
    pub id: String,
    /// Always `true` — see [`ApplyResponse::applied`].
    pub discarded: bool,
}

/// Every `tune`/`bandit` run that is still on offer: not applied, not
/// discarded, and proposing at least one knob the live config does not
/// already use. Newest-first (`eval_runs::list`'s order).
pub fn list(ctx: &mut Ctx<'_>) -> Result<Vec<Proposal>> {
    if !ctx.paths.db_path().exists() {
        return Ok(Vec::new());
    }
    let cfg = ctx.cfg;
    let conn = ctx.conn()?;
    let mut out = Vec::new();
    for row in eval_runs::list(conn, ALL_RUNS)? {
        if !is_open_search(&row) {
            continue;
        }
        let Some(candidate) = parse_knobs(&row) else {
            continue;
        };
        let knobs = diff(cfg, &candidate)?;
        if knobs.is_empty() {
            continue;
        }
        out.push(proposal(row, knobs));
    }
    Ok(out)
}

/// Write one proposal's knobs into `config.toml` and stamp the run
/// `applied`. Refuses an unknown id (`404`), a run that is not an open
/// knob search (`400`: already applied, discarded, or a plain `eval`), and
/// a run whose stored `knobs` is not a full knob set (`400`).
///
/// The caller reloads the server's in-memory config afterwards; this
/// function only owns the file and the row.
pub fn apply(ctx: &mut Ctx<'_>, id: &str) -> Result<ApplyResponse> {
    let config_file = ctx.paths.config_file();
    let conn = ctx.conn()?;
    let row = fetch_open(conn, id)?;
    let candidate = require_knobs(&row)?;
    tune::apply_to_config_file(&config_file, &candidate)?;
    eval_runs::set_applied(conn, id)?;
    Ok(ApplyResponse {
        id: row.id,
        applied: true,
        config_file: config_file.to_string_lossy().into_owned(),
    })
}

/// Stamp a run `discarded` so it stops being offered. Touches no config
/// file. Refuses an unknown id (`404`) or an already-applied run (`400` —
/// discarding a knob set that is already live would misreport history);
/// discarding an already-discarded run is idempotent.
pub fn discard(ctx: &mut Ctx<'_>, id: &str) -> Result<DiscardResponse> {
    let conn = ctx.conn()?;
    let row = eval_runs::get(conn, id)?.ok_or_else(|| not_found(id))?;
    if row.applied {
        return Err(Error::BadRequest(format!(
            "eval run {id} was already applied and cannot be discarded"
        )));
    }
    eval_runs::set_discarded(conn, id)?;
    Ok(DiscardResponse {
        id: row.id,
        discarded: true,
    })
}

/// A `tune`/`bandit` run that has been neither applied nor discarded.
fn is_open_search(row: &EvalRunRow) -> bool {
    matches!(row.kind.as_str(), "tune" | "bandit") && !row.applied && !row.discarded
}

/// Read one row and require it to be an open knob search.
fn fetch_open(conn: &rusqlite::Connection, id: &str) -> Result<EvalRunRow> {
    let row = eval_runs::get(conn, id)?.ok_or_else(|| not_found(id))?;
    if !is_open_search(&row) {
        return Err(Error::BadRequest(format!(
            "eval run {id} is not an open proposal (kind={}, applied={}, discarded={})",
            row.kind, row.applied, row.discarded
        )));
    }
    Ok(row)
}

/// The `404` for an id with no `eval_runs` row.
fn not_found(id: &str) -> Error {
    Error::NotFound(format!("eval run {id}"))
}

/// Parse a row's stored `knobs` JSON as a full knob set, or `None`. A run
/// recorded by an older binary (or by a future one with a different knob
/// shape) is simply not offered as a proposal rather than failing the whole
/// listing.
fn parse_knobs(row: &EvalRunRow) -> Option<TuneCandidate> {
    match serde_json::from_value::<TuneCandidate>(row.knobs.clone()) {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::debug!(id = %row.id, error = %e, "eval_runs.knobs is not a knob set");
            None
        }
    }
}

/// [`parse_knobs`] for the apply path, where an unusable knob set is a
/// caller-visible `400` rather than a silently skipped row.
fn require_knobs(row: &EvalRunRow) -> Result<TuneCandidate> {
    serde_json::from_value::<TuneCandidate>(row.knobs.clone()).map_err(|e| {
        Error::BadRequest(format!(
            "eval run {} does not record a full knob set: {e}",
            row.id
        ))
    })
}

/// Assemble a [`Proposal`] from its row and its (non-empty) diff.
fn proposal(row: EvalRunRow, knobs: Vec<KnobChange>) -> Proposal {
    Proposal {
        id: row.id,
        kind: row.kind,
        at: row.at,
        recall_at_k: row.recall,
        mrr: row.mrr,
        golden_pairs: row.golden_pairs,
        k: row.k,
        knobs,
    }
}

/// One `(name, from, to)` entry, JSON-encoded so a client renders numbers,
/// pairs, and integers without a per-knob schema.
fn change<T: Serialize>(name: &'static str, from: T, to: T) -> Result<KnobChange> {
    Ok(KnobChange {
        name,
        from: serde_json::to_value(from).map_err(Error::Json)?,
        to: serde_json::to_value(to).map_err(Error::Json)?,
    })
}

/// The per-knob diff between the live config and a candidate. Floats are
/// compared with `total_cmp` (exact bit ordering) rather than `==`: the two
/// values are stored numbers being checked for identity, not measurements
/// being checked for closeness.
fn diff(cfg: &Config, c: &TuneCandidate) -> Result<Vec<KnobChange>> {
    let mut out = Vec::new();
    let r = &cfg.retrieval;
    if r.rrf_k.total_cmp(&c.rrf_k).is_ne() {
        out.push(change("rrf_k", r.rrf_k, c.rrf_k)?);
    }
    if cfg.rank.decay.total_cmp(&c.decay).is_ne() {
        out.push(change("decay", cfg.rank.decay, c.decay)?);
    }
    if cfg.rank.mmr_lambda.total_cmp(&c.mmr_lambda).is_ne() {
        out.push(change("mmr_lambda", cfg.rank.mmr_lambda, c.mmr_lambda)?);
    }
    if r.bm25_weights.0.total_cmp(&c.bm25_weights.0).is_ne()
        || r.bm25_weights.1.total_cmp(&c.bm25_weights.1).is_ne()
    {
        out.push(change("bm25_weights", r.bm25_weights, c.bm25_weights)?);
    }
    if r.graph_hops != c.graph_hops {
        out.push(change("graph_hops", r.graph_hops, c.graph_hops)?);
    }
    if r.graph_seeds != c.graph_seeds {
        out.push(change("graph_seeds", r.graph_seeds, c.graph_seeds)?);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests/learning_proposals.rs"]
mod tests;
