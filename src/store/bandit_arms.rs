//! `bandit_arms` row CRUD: the Beta-posterior state per `comemory bandit`
//! arm. The knob grid, Thompson sampling, and the win/loss decision stay in
//! [`crate::domains::learning::evaluation::bandit`] — this module owns only the SQL text and its row
//! mapping.

use rusqlite::Connection;

use super::{
    orm,
    schema_learning::{BanditArms, bandit_arms as col},
};
use crate::prelude::*;
use toolu_orm::core::query_column::CommonOps;

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
    orm::execute(
        conn,
        BanditArms::insert()
            .set(&col::arm_id, arm.arm_id)
            .set(&col::rrf_k, arm.rrf_k)
            .set(&col::decay, arm.decay)
            .set(&col::mmr_lambda, arm.mmr_lambda)
            .set(&col::bm25_body, arm.bm25_body)
            .set(&col::bm25_tags, arm.bm25_tags)
            .set(&col::alpha, 1.0)
            .set(&col::beta, 1.0)
            .set(&col::pulls, 0)
            .set_null(&col::last_mrr)
            .set(&col::updated_at, arm.at)
            .or_ignore()
            .to_sql(),
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
    orm::query_optional(
        conn,
        BanditArms::select()
            .columns_typed(&[&col::alpha, &col::beta, &col::pulls, &col::last_mrr])
            .filter(col::arm_id.eq(arm_id))
            .to_sql(),
        |r| {
            Ok(ArmState {
                alpha: r.get(0)?,
                beta: r.get(1)?,
                pulls: r.get(2)?,
                last_mrr: r.get(3)?,
            })
        },
    )
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
    let query = BanditArms::update()
        .set_expr(&col::pulls, "pulls + 1")
        .set(&col::last_mrr, mrr)
        .set(&col::updated_at, at)
        .filter(col::arm_id.eq(arm_id));
    let query = if won {
        query.set_expr(&col::alpha, "alpha + 1.0")
    } else {
        query.set_expr(&col::beta, "beta + 1.0")
    };
    orm::execute(conn, query.to_sql())?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/bandit_arms.rs"]
mod tests;
