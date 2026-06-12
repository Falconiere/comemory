//! Deterministic grid search over blend weights, scored by eval MRR
//! (recall@k tie-break) on the merged golden set.

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::config::file::TuneConfig;
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

impl TuneReport {
    /// The top-ranked candidate. Errors only on an empty ranking, which
    /// [`run_tune`] can never produce (`Config::validate` rejects empty
    /// `[tune]` grid lists, so the cartesian product has >= 1 point).
    pub fn winner(&self) -> Result<&ScoredCandidate> {
        self.ranked
            .first()
            .ok_or_else(|| Error::Other("tune produced an empty candidate ranking".into()))
    }

    /// True when the winner *strictly* beats the baseline: higher mrr,
    /// or exactly-equal mrr with strictly higher recall@k. Ties never
    /// count as an improvement, so `comemory tune --apply` cannot churn
    /// `config.toml` when the grid merely matches the current knobs.
    pub fn improves_baseline(&self) -> bool {
        let Ok(w) = self.winner() else {
            return false;
        };
        match w.mrr.total_cmp(&self.baseline.mrr) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Equal => w.recall_at_k > self.baseline.recall_at_k,
            std::cmp::Ordering::Less => false,
        }
    }
}

/// Resolve the minimum-golden-pairs floor: `COMEMORY_TUNE_MIN_GOLDEN`
/// when set (a test hook, documented as such — an invalid value is a
/// hard error naming the variable), else [`MIN_GOLDEN_PAIRS`]. Lives
/// next to the constant it overrides so the policy has one home.
pub fn resolve_min_pairs() -> Result<usize> {
    Ok(
        crate::config::env::env_parse::<usize>("COMEMORY_TUNE_MIN_GOLDEN")?
            .unwrap_or(MIN_GOLDEN_PAIRS),
    )
}

/// The cartesian product of the configured grid lists (`[tune]` in
/// config.toml). The defaults reproduce the M1 3×3×3×3 = 81-point grid;
/// `Config::validate` guarantees every list is non-empty and every value
/// passes its scalar knob's bounds, so the product is never empty.
pub fn grid(t: &TuneConfig) -> Vec<TuneCandidate> {
    let cap = t.rrf_k_grid.len() * t.decay_grid.len() * t.mmr_lambda_grid.len() * t.bm25_grid.len();
    let mut out = Vec::with_capacity(cap);
    for &rrf_k in &t.rrf_k_grid {
        for &decay in &t.decay_grid {
            for &mmr_lambda in &t.mmr_lambda_grid {
                for &bm25_weights in &t.bm25_grid {
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
    let candidates = grid(&base.tune);
    if pairs.len() < min_pairs {
        return Err(Error::Unavailable(format!(
            "tune needs >= {min_pairs} golden pairs (have {}): grid-searching {} configs \
             against a thin set is overfitting, not tuning",
            pairs.len(),
            candidates.len()
        )));
    }
    let baseline_candidate = TuneCandidate {
        rrf_k: base.retrieval.rrf_k,
        decay: base.rank.decay,
        mmr_lambda: base.rank.mmr_lambda,
        bm25_weights: base.retrieval.bm25_weights,
    };
    let baseline = score(&runner::run_eval(base, conn, pairs, k)?, baseline_candidate);
    let mut ranked = Vec::with_capacity(candidates.len());
    // `rrf_k` only feeds the hybrid fusion arm, and eval replay is
    // lexical-only (BYO vectors cannot be replayed offline) — so two grid
    // points differing only in rrf_k always score identically. Memoize on
    // the knobs that actually reach the lexical path; with the default
    // grid, 54 of the 81 points reuse a cached (mrr, recall@k) pair
    // instead of re-running the whole golden set.
    let mut cache: std::collections::HashMap<(u64, u64, u32, u32), (f64, f64)> =
        std::collections::HashMap::new();
    for c in candidates {
        let key = (
            c.decay.to_bits(),
            c.mmr_lambda.to_bits(),
            c.bm25_weights.0.to_bits(),
            c.bm25_weights.1.to_bits(),
        );
        let (mrr, recall_at_k) = match cache.get(&key) {
            Some(&cached) => cached,
            None => {
                let report = runner::run_eval(&with_candidate(base, &c), conn, pairs, k)?;
                cache.insert(key, (report.mrr, report.recall_at_k));
                (report.mrr, report.recall_at_k)
            }
        };
        ranked.push(ScoredCandidate {
            candidate: c,
            mrr,
            recall_at_k,
        });
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
