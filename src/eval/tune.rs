//! Deterministic grid search over blend weights, scored by eval MRR
//! (recall@k tie-break) on the merged golden set.

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::eval::golden::GoldenPair;
use crate::eval::runner::{self, EvalReport};
use crate::prelude::*;

/// Minimum golden pairs before tuning is statistically honest.
/// Overridable via `COMEMORY_TUNE_MIN_GOLDEN` (a test hook, documented
/// as such — not a tuning knob).
pub const MIN_GOLDEN_PAIRS: usize = 10;

/// One grid point.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct TuneCandidate {
    /// RRF fusion constant.
    pub rrf_k: f32,
    /// ACT-R decay exponent.
    pub decay: f64,
    /// MMR relevance-vs-diversity lambda.
    pub mmr_lambda: f64,
    /// BM25 (body, tags) weights.
    pub bm25_weights: (f32, f32),
}

/// One scored grid point.
#[derive(Debug, Serialize)]
pub struct ScoredCandidate {
    /// The parameters evaluated.
    pub candidate: TuneCandidate,
    /// Mean MRR on the golden set (primary criterion).
    pub mrr: f64,
    /// Mean recall@k (tie-break).
    pub recall_at_k: f64,
}

/// Tune report: every candidate scored, best first, plus the baseline
/// (current config) score for the delta.
#[derive(Debug, Serialize)]
pub struct TuneReport {
    /// k used for recall@k.
    pub k: usize,
    /// Golden pairs evaluated per candidate.
    pub golden_pairs: usize,
    /// Score of the *current* configuration.
    pub baseline: ScoredCandidate,
    /// All candidates, sorted best-first (mrr desc, recall desc, then
    /// candidate field order for full determinism).
    pub ranked: Vec<ScoredCandidate>,
}

/// The 3×3×3×3 = 81-point grid. Values bracket the M1 defaults.
pub fn grid() -> Vec<TuneCandidate> {
    let mut out = Vec::with_capacity(81);
    for &rrf_k in &[20.0f32, 60.0, 100.0] {
        for &decay in &[0.3f64, 0.5, 0.8] {
            for &mmr_lambda in &[0.5f64, 0.7, 0.9] {
                for &bm25_weights in &[(1.0f32, 3.0f32), (1.0, 1.0), (2.0, 1.0)] {
                    out.push(TuneCandidate {
                        rrf_k,
                        decay,
                        mmr_lambda,
                        bm25_weights,
                    });
                }
            }
        }
    }
    out
}

/// Clone `base` with the candidate's four knobs swapped in.
fn with_candidate(base: &Config, c: &TuneCandidate) -> Config {
    let mut cfg = base.clone();
    cfg.retrieval.rrf_k = c.rrf_k;
    cfg.retrieval.bm25_weights = c.bm25_weights;
    cfg.rank.decay = c.decay;
    cfg.rank.mmr_lambda = c.mmr_lambda;
    cfg
}

/// Lift the aggregate metrics out of an [`EvalReport`] for one candidate.
fn score(report: &EvalReport, c: TuneCandidate) -> ScoredCandidate {
    ScoredCandidate {
        candidate: c,
        mrr: report.mrr,
        recall_at_k: report.recall_at_k,
    }
}

/// Run the full grid (plus the baseline) against the golden set.
/// Refuses with [`Error::Unavailable`] below the honesty floor.
pub fn run_tune(
    base: &Config,
    conn: &Connection,
    pairs: &[GoldenPair],
    k: usize,
    min_pairs: usize,
) -> Result<TuneReport> {
    if pairs.len() < min_pairs {
        return Err(Error::Unavailable(format!(
            "tune needs >= {min_pairs} golden pairs (have {}): grid-searching 81 configs \
             against a thin set is overfitting, not tuning",
            pairs.len()
        )));
    }
    let baseline_candidate = TuneCandidate {
        rrf_k: base.retrieval.rrf_k,
        decay: base.rank.decay,
        mmr_lambda: base.rank.mmr_lambda,
        bm25_weights: base.retrieval.bm25_weights,
    };
    let baseline = score(&runner::run_eval(base, conn, pairs, k)?, baseline_candidate);
    let mut ranked = Vec::with_capacity(81);
    for c in grid() {
        let report = runner::run_eval(&with_candidate(base, &c), conn, pairs, k)?;
        ranked.push(score(&report, c));
    }
    ranked.sort_by(|a, b| {
        b.mrr
            .total_cmp(&a.mrr)
            .then_with(|| b.recall_at_k.total_cmp(&a.recall_at_k))
            .then_with(|| a.candidate.rrf_k.total_cmp(&b.candidate.rrf_k))
            .then_with(|| a.candidate.decay.total_cmp(&b.candidate.decay))
            .then_with(|| a.candidate.mmr_lambda.total_cmp(&b.candidate.mmr_lambda))
            .then_with(|| {
                a.candidate
                    .bm25_weights
                    .0
                    .total_cmp(&b.candidate.bm25_weights.0)
            })
    });
    Ok(TuneReport {
        k,
        golden_pairs: pairs.len(),
        baseline,
        ranked,
    })
}

/// Write the winner's four knobs into `config.toml`, preserving every
/// other key. Atomic tmp + rename (same pattern as memory save).
/// CAVEAT: round-trips through `toml::Value`, so comments in an
/// existing file are lost — documented in the CLI help.
pub fn apply_to_config_file(path: &Path, w: &TuneCandidate) -> Result<()> {
    let mut root: toml::Value = if path.exists() {
        let raw = std::fs::read_to_string(path).map_err(Error::Io)?;
        toml::from_str(&raw).map_err(|e| Error::Config(format!("config.toml: {e}")))?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let table = root
        .as_table_mut()
        .ok_or_else(|| Error::Config("config.toml: root is not a table".into()))?;
    {
        let retrieval = section(table, "retrieval")?;
        retrieval.insert("rrf_k".into(), toml::Value::Float(f64::from(w.rrf_k)));
        retrieval.insert(
            "bm25_weights".into(),
            toml::Value::Array(vec![
                toml::Value::Float(f64::from(w.bm25_weights.0)),
                toml::Value::Float(f64::from(w.bm25_weights.1)),
            ]),
        );
    }
    {
        let rank = section(table, "rank")?;
        rank.insert("decay".into(), toml::Value::Float(w.decay));
        rank.insert("mmr_lambda".into(), toml::Value::Float(w.mmr_lambda));
    }
    let rendered = toml::to_string_pretty(&root)
        .map_err(|e| Error::Config(format!("config.toml render: {e}")))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, rendered).map_err(Error::Io)?;
    std::fs::rename(&tmp, path).map_err(Error::Io)?;
    Ok(())
}

/// Fetch-or-create a named sub-table of `table`. Errors when the key
/// exists but is not a table (a malformed config must not be silently
/// overwritten).
fn section<'t>(
    table: &'t mut toml::map::Map<String, toml::Value>,
    name: &str,
) -> Result<&'t mut toml::map::Map<String, toml::Value>> {
    table
        .entry(name)
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| Error::Config(format!("config.toml: [{name}] is not a table")))
}
