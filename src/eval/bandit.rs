//! Eval-gated Thompson bandit over the `[tune]` discrete grid.
//!
//! Arms are the cartesian product of `TuneConfig` grids. Report ranks by
//! posterior mean; `--apply` Thompson-samples one arm (deterministic seed),
//! confirms via offline eval, and writes `config.toml` only when
//! [`crate::eval::tune::beats_baseline`] is true.

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::config::Config;
use crate::eval::bandit_rng::{SplitMix64, sample_beta};
use crate::eval::golden::GoldenPair;
use crate::eval::runner;
use crate::eval::tune::{self, TuneCandidate};
use crate::prelude::*;
use crate::store::memory_row;

/// One bandit arm with Beta posterior.
#[derive(Debug, Clone, Serialize)]
pub struct Arm {
    /// Stable id (16-hex SHA-256 of knob bit patterns).
    pub arm_id: String,
    /// Knobs this arm represents.
    pub candidate: TuneCandidate,
    /// Beta α (wins + prior).
    pub alpha: f64,
    /// Beta β (losses + prior).
    pub beta: f64,
    /// Confirm cycles that updated this arm.
    pub pulls: i64,
    /// Last observed MRR (if any).
    pub last_mrr: Option<f64>,
}

impl Arm {
    /// Posterior mean α / (α + β).
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }
}

/// Report from `comemory bandit` (with or without `--apply`).
#[derive(Debug, Serialize)]
pub struct BanditReport {
    /// k used for recall@k during confirm.
    pub k: usize,
    /// Golden pairs evaluated.
    pub golden_pairs: usize,
    /// Current-config baseline MRR.
    pub baseline_mrr: f64,
    /// Current-config baseline recall@k.
    pub baseline_recall_at_k: f64,
    /// Arms eligible under the current grid, mean-desc.
    pub ranked: Vec<Arm>,
    /// Thompson-sampled arm for this run.
    pub proposed: TuneCandidate,
    /// Whether config.toml was rewritten.
    pub applied: bool,
}

/// Stable 16-hex arm id from little-endian `to_bits` of the five knobs.
pub fn arm_id(c: &TuneCandidate) -> String {
    let mut h = Sha256::new();
    h.update(c.rrf_k.to_bits().to_le_bytes());
    h.update(c.decay.to_bits().to_le_bytes());
    h.update(c.mmr_lambda.to_bits().to_le_bytes());
    h.update(c.bm25_weights.0.to_bits().to_le_bytes());
    h.update(c.bm25_weights.1.to_bits().to_le_bytes());
    let dig = h.finalize();
    let mut out = String::with_capacity(16);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &b in &dig[..8] {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// Insert missing arms for the current `[tune]` grid (Beta(1,1) priors).
pub fn seed_arms(conn: &Connection, cfg: &Config, at: &str) -> Result<()> {
    for c in tune::grid(&cfg.tune) {
        let id = arm_id(&c);
        conn.execute(
            "INSERT OR IGNORE INTO bandit_arms(\
                 arm_id, rrf_k, decay, mmr_lambda, bm25_body, bm25_tags, \
                 alpha, beta, pulls, last_mrr, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1.0, 1.0, 0, NULL, ?7)",
            rusqlite::params![
                id,
                f64::from(c.rrf_k),
                c.decay,
                c.mmr_lambda,
                f64::from(c.bm25_weights.0),
                f64::from(c.bm25_weights.1),
                at,
            ],
        )?;
    }
    Ok(())
}

/// Load arms in the current grid, ranked by posterior mean then knob order.
pub fn load_ranked(conn: &Connection, cfg: &Config) -> Result<Vec<Arm>> {
    let grid = tune::grid(&cfg.tune);
    let mut out = Vec::with_capacity(grid.len());
    for c in &grid {
        let id = arm_id(c);
        match conn.query_row(
            "SELECT alpha, beta, pulls, last_mrr FROM bandit_arms WHERE arm_id = ?1",
            [&id],
            |r| {
                Ok(Arm {
                    arm_id: id.clone(),
                    candidate: *c,
                    alpha: r.get(0)?,
                    beta: r.get(1)?,
                    pulls: r.get(2)?,
                    last_mrr: r.get(3)?,
                })
            },
        ) {
            Ok(arm) => out.push(arm),
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(e) => return Err(Error::Sqlite(e)),
        }
    }
    out.sort_by(|a, b| {
        b.mean()
            .total_cmp(&a.mean())
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
    Ok(out)
}

/// Deterministic seed from golden size + arm count + schema version.
pub fn sample_seed(golden_pairs: usize, arm_count: usize) -> u64 {
    let mut h = Sha256::new();
    h.update(golden_pairs.to_le_bytes());
    h.update(arm_count.to_le_bytes());
    h.update(crate::store::migrate::CURRENT_VERSION.as_bytes());
    let dig = h.finalize();
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&dig[..8]);
    u64::from_le_bytes(buf)
}

/// Thompson-sample one arm; tie-break by TuneCandidate field order.
pub fn thompson_sample(arms: &[Arm], seed: u64) -> Result<&Arm> {
    if arms.is_empty() {
        return Err(Error::Other("bandit: no arms to sample".into()));
    }
    let mut rng = SplitMix64::new(seed);
    let mut best_i = 0usize;
    let mut best_s = f64::NEG_INFINITY;
    for (i, arm) in arms.iter().enumerate() {
        let s = sample_beta(&mut rng, arm.alpha, arm.beta);
        // sample_beta already returns finite; 0.5 == Beta(1,1) mean if not.
        let s = if s.is_finite() { s } else { 0.5 };
        if s > best_s
            || (s == best_s
                && arm.candidate.rrf_k.total_cmp(&arms[best_i].candidate.rrf_k)
                    == std::cmp::Ordering::Less)
        {
            best_i = i;
            best_s = s;
        }
    }
    Ok(&arms[best_i])
}

/// Update posterior after confirm; `won` is [`tune::beats_baseline`].
pub fn record_outcome(
    conn: &Connection,
    arm_id: &str,
    won: bool,
    mrr: f64,
    at: &str,
) -> Result<()> {
    let sql = if won {
        "UPDATE bandit_arms SET alpha = alpha + 1.0, pulls = pulls + 1, \
             last_mrr = ?2, updated_at = ?3 WHERE arm_id = ?1"
    } else {
        "UPDATE bandit_arms SET beta = beta + 1.0, pulls = pulls + 1, \
             last_mrr = ?2, updated_at = ?3 WHERE arm_id = ?1"
    };
    conn.execute(sql, rusqlite::params![arm_id, mrr, at])?;
    Ok(())
}

/// Seed, sample, confirm vs baseline; write `config_path` when `apply` and win.
pub fn run_bandit(
    cfg: &Config,
    conn: &mut Connection,
    pairs: &[GoldenPair],
    k: usize,
    min_pairs: usize,
    apply: bool,
    config_path: &Path,
) -> Result<BanditReport> {
    if pairs.len() < min_pairs {
        return Err(Error::Unavailable(format!(
            "bandit needs >= {min_pairs} golden pairs (have {}): sampling knobs \
             against a thin set is overfitting, not learning",
            pairs.len(),
        )));
    }
    let at = memory_row::iso_format(OffsetDateTime::now_utc())?;
    seed_arms(conn, cfg, &at)?;
    // Drop the ranked borrow before confirm: run_eval needs &mut Connection.
    let proposed = {
        let ranked = load_ranked(conn, cfg)?;
        let seed = sample_seed(pairs.len(), ranked.len());
        thompson_sample(&ranked, seed)?.candidate
    };

    let baseline = runner::run_eval(cfg, conn, pairs, k)?;
    let cand = runner::run_eval(&tune::with_candidate(cfg, &proposed), conn, pairs, k)?;
    let won = tune::beats_baseline(
        cand.mrr,
        cand.recall_at_k,
        baseline.mrr,
        baseline.recall_at_k,
    );
    record_outcome(conn, &arm_id(&proposed), won, cand.mrr, &at)?;

    let applied = apply && won;
    if applied {
        tune::apply_to_config_file(config_path, &proposed)?;
    }

    // Reload so the report's ranked posteriors include this run's outcome.
    Ok(BanditReport {
        k,
        golden_pairs: pairs.len(),
        baseline_mrr: baseline.mrr,
        baseline_recall_at_k: baseline.recall_at_k,
        ranked: load_ranked(conn, cfg)?,
        proposed,
        applied,
    })
}
