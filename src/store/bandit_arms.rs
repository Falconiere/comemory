//! `bandit_arms` row CRUD: the Beta-posterior state per `comemory bandit`
//! arm. The knob grid, Thompson sampling, and the win/loss decision stay in
//! [`crate::eval::bandit`] — this module owns only the SQL text and its row
//! mapping.

use rusqlite::{Connection, OptionalExtension, params};

use crate::prelude::*;

/// A freshly-seeded arm's knob values, bundled into a struct rather than
/// seven positional arguments (`clippy::too_many_arguments`).
pub struct NewArm<'a> {
    /// Stable 16-hex arm id (`eval::bandit::arm_id`).
    pub arm_id: &'a str,
    /// `[tune].rrf_k` value this arm represents.
    pub rrf_k: f64,
    /// `[tune].decay` value.
    pub decay: f64,
    /// `[tune].mmr_lambda` value.
    pub mmr_lambda: f64,
    /// `[tune].bm25_weights.0` (body weight).
    pub bm25_body: f64,
    /// `[tune].bm25_weights.1` (tags weight).
    pub bm25_tags: f64,
    /// ISO-8601 UTC timestamp this arm was seeded at.
    pub at: &'a str,
}

/// Insert one arm with Beta(1,1) priors, or no-op if it already exists
/// (`INSERT OR IGNORE`) — a re-seed of an already-pulled arm must not reset
/// its posterior.
pub fn seed(conn: &Connection, arm: &NewArm<'_>) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO bandit_arms(\
             arm_id, rrf_k, decay, mmr_lambda, bm25_body, bm25_tags, \
             alpha, beta, pulls, last_mrr, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1.0, 1.0, 0, NULL, ?7)",
        params![
            arm.arm_id,
            arm.rrf_k,
            arm.decay,
            arm.mmr_lambda,
            arm.bm25_body,
            arm.bm25_tags,
            arm.at,
        ],
    )?;
    Ok(())
}

/// The stored Beta-posterior state of one arm.
pub struct ArmState {
    /// Beta α (wins + prior).
    pub alpha: f64,
    /// Beta β (losses + prior).
    pub beta: f64,
    /// Confirm cycles that updated this arm.
    pub pulls: i64,
    /// Last observed MRR, if any.
    pub last_mrr: Option<f64>,
}

/// Load the stored state of `arm_id`, or `None` when the arm has no
/// `bandit_arms` row yet (never seeded, or seeded under a pre-widening hash).
pub fn load(conn: &Connection, arm_id: &str) -> Result<Option<ArmState>> {
    conn.query_row(
        "SELECT alpha, beta, pulls, last_mrr FROM bandit_arms WHERE arm_id = ?1",
        [arm_id],
        |r| {
            Ok(ArmState {
                alpha: r.get(0)?,
                beta: r.get(1)?,
                pulls: r.get(2)?,
                last_mrr: r.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Error::Sqlite)
}

/// Update one arm's posterior after a confirm cycle: `alpha += 1.0` on a win,
/// `beta += 1.0` on a loss, plus `pulls += 1` and the latest `last_mrr` /
/// `updated_at` either way.
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
    conn.execute(sql, params![arm_id, mrr, at])?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/bandit_arms.rs"]
mod tests;
