//! Deterministic search over blend weights, scored by eval MRR (recall@k
//! tie-break) on the merged golden set — the exhaustive `[tune]` grid, or a
//! seeded uniform sample of it when `tune.samples > 0` (the sampler itself
//! lives in [`crate::eval::tune_sample`]).

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::config::TuneConfig;
use crate::eval::golden::GoldenPair;
use crate::eval::runner::{self, EvalReport};
use crate::eval::tune_sample;
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
    /// Graph-expansion hop depth (`0` disables the leg).
    pub graph_hops: u32,
    /// Provisional top hits seeding the graph-expansion walk.
    pub graph_seeds: usize,
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
        beats_baseline(
            w.mrr,
            w.recall_at_k,
            self.baseline.mrr,
            self.baseline.recall_at_k,
        )
    }
}

/// Strict improvement predicate shared by `tune --apply` and `bandit --apply`.
pub fn beats_baseline(cand_mrr: f64, cand_recall: f64, base_mrr: f64, base_recall: f64) -> bool {
    match cand_mrr.total_cmp(&base_mrr) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => cand_recall > base_recall,
        std::cmp::Ordering::Less => false,
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
/// config.toml). The defaults reproduce the M1 3×3×3×3 grid widened by the
/// two graph knobs — 3^6 = 729 points; `Config::validate` guarantees every
/// list is non-empty and every value passes its scalar knob's bounds, so
/// the product is never empty.
///
/// The two graph dimensions are the OUTERMOST loops, so within each
/// `(graph_hops, graph_seeds)` block the legacy four enumerate in exactly
/// their pre-F5 order: singleton graph grids reproduce the historical
/// 81-candidate sequence projected onto the legacy fields.
pub fn grid(t: &TuneConfig) -> Vec<TuneCandidate> {
    let cap = t.graph_hops_grid.len()
        * t.graph_seeds_grid.len()
        * t.rrf_k_grid.len()
        * t.decay_grid.len()
        * t.mmr_lambda_grid.len()
        * t.bm25_grid.len();
    let mut out = Vec::with_capacity(cap);
    for &graph_hops in &t.graph_hops_grid {
        for &graph_seeds in &t.graph_seeds_grid {
            for &rrf_k in &t.rrf_k_grid {
                for &decay in &t.decay_grid {
                    for &mmr_lambda in &t.mmr_lambda_grid {
                        for &bm25_weights in &t.bm25_grid {
                            out.push(TuneCandidate {
                                rrf_k,
                                decay,
                                mmr_lambda,
                                bm25_weights,
                                graph_hops,
                                graph_seeds,
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

/// Clone `base` with the candidate's six knobs swapped in.
pub(crate) fn with_candidate(base: &Config, c: &TuneCandidate) -> Config {
    let mut cfg = base.clone();
    cfg.retrieval.rrf_k = c.rrf_k;
    cfg.retrieval.bm25_weights = c.bm25_weights;
    cfg.retrieval.graph_hops = c.graph_hops;
    cfg.retrieval.graph_seeds = c.graph_seeds;
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

/// Candidates for one run: the exhaustive [`grid`] when `tune.samples` is
/// `0`, else a seeded uniform sample of it. An explicit `seed` (from
/// `tune --seed`) overrides the derived default, so a run is reproducible
/// either way.
fn candidates_for(t: &TuneConfig, pairs: usize, seed: Option<u64>) -> Vec<TuneCandidate> {
    if t.samples == 0 {
        return grid(t);
    }
    let sizes = tune_sample::pool_sizes(t);
    let seed = seed.unwrap_or_else(|| tune_sample::sample_seed(pairs, &sizes));
    tune_sample::sample_candidates(t, seed)
}

/// Memoization key for [`score_all`]: bit patterns of the six knobs that
/// reach the lexical replay path, in this exact order —
/// `(decay, mmr_lambda, bm25_weights.0, bm25_weights.1, graph_hops,
/// graph_seeds)` — every [`TuneCandidate`] field except `rrf_k`, with the
/// `bm25_weights` pair contributing both of its components. `rrf_k` is dead
/// on the lexical path and deliberately absent, so rrf_k-only variants
/// collapse onto one cache entry.
type ScoreKey = (u64, u64, u32, u32, u32, u64);

/// Score every candidate against the golden set.
///
/// `rrf_k` only feeds the hybrid fusion arm, and eval replay is
/// lexical-only (BYO vectors cannot be replayed offline) — so two
/// candidates differing only in rrf_k always score identically. Memoize on
/// the six knobs that do reach the lexical path (the graph pair among them:
/// `expand_and_fuse` runs unconditionally in `router::route`), which lets
/// rrf_k-only variants reuse a cached `(mrr, recall@k)` pair instead of
/// re-running the whole golden set.
fn score_all(
    base: &Config,
    conn: &Connection,
    pairs: &[GoldenPair],
    k: usize,
    candidates: Vec<TuneCandidate>,
) -> Result<Vec<ScoredCandidate>> {
    let mut ranked = Vec::with_capacity(candidates.len());
    let mut cache: std::collections::HashMap<ScoreKey, (f64, f64)> =
        std::collections::HashMap::new();
    for c in candidates {
        let key = (
            c.decay.to_bits(),
            c.mmr_lambda.to_bits(),
            c.bm25_weights.0.to_bits(),
            c.bm25_weights.1.to_bits(),
            c.graph_hops,
            c.graph_seeds as u64,
        );
        let (mrr, recall_at_k) = if let Some(&cached) = cache.get(&key) {
            cached
        } else {
            let report = runner::run_eval(&with_candidate(base, &c), conn, pairs, k)?;
            cache.insert(key, (report.mrr, report.recall_at_k));
            (report.mrr, report.recall_at_k)
        };
        ranked.push(ScoredCandidate {
            candidate: c,
            mrr,
            recall_at_k,
        });
    }
    Ok(ranked)
}

/// Best-first order, pinned to a total order over the scores and all seven
/// knob fields so a ranking never depends on candidate arrival order.
fn sort_ranked(ranked: &mut [ScoredCandidate]) {
    ranked.sort_by(|a, b| {
        let (x, y) = (&a.candidate, &b.candidate);
        b.mrr
            .total_cmp(&a.mrr)
            .then_with(|| b.recall_at_k.total_cmp(&a.recall_at_k))
            .then_with(|| x.rrf_k.total_cmp(&y.rrf_k))
            .then_with(|| x.decay.total_cmp(&y.decay))
            .then_with(|| x.mmr_lambda.total_cmp(&y.mmr_lambda))
            .then_with(|| x.bm25_weights.0.total_cmp(&y.bm25_weights.0))
            .then_with(|| x.graph_hops.cmp(&y.graph_hops))
            .then_with(|| x.graph_seeds.cmp(&y.graph_seeds))
    });
}

/// Run the candidate set (plus the baseline) against the golden set.
/// Refuses with [`Error::Unavailable`] below the honesty floor.
pub fn run_tune(
    base: &Config,
    conn: &Connection,
    pairs: &[GoldenPair],
    k: usize,
    min_pairs: usize,
    seed: Option<u64>,
) -> Result<TuneReport> {
    let candidates = candidates_for(&base.tune, pairs.len(), seed);
    if pairs.len() < min_pairs {
        return Err(Error::Unavailable(format!(
            "tune needs >= {min_pairs} golden pairs (have {}): searching {} configs \
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
        graph_hops: base.retrieval.graph_hops,
        graph_seeds: base.retrieval.graph_seeds,
    };
    let baseline = score(&runner::run_eval(base, conn, pairs, k)?, baseline_candidate);
    let mut ranked = score_all(base, conn, pairs, k, candidates)?;
    sort_ranked(&mut ranked);
    Ok(TuneReport {
        k,
        golden_pairs: pairs.len(),
        baseline,
        ranked,
    })
}

/// Write the winner's six knobs into `config.toml`, preserving every
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
        retrieval.insert(
            "graph_hops".into(),
            toml::Value::Integer(i64::from(w.graph_hops)),
        );
        retrieval.insert(
            "graph_seeds".into(),
            toml::Value::Integer(w.graph_seeds as i64),
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

#[cfg(test)]
#[path = "tests/tune.rs"]
mod tests;
