//! `candidate_judgments` row CRUD: one reviewed relevance verdict per
//! candidate per captured observation (#209).
//!
//! Keyed by `(observation_id, candidate_ref)`, so a verdict can only ever name
//! a candidate that observation actually recorded, and re-judging the same
//! candidate replaces the row rather than accumulating verdicts.

use std::collections::HashMap;

use rusqlite::Connection;

use super::{
    orm,
    schema_learning::{CandidateJudgments, candidate_judgments as col},
};
use crate::prelude::*;
use toolu_orm::core::query_column::CommonOps;

/// Insert parameters for one `candidate_judgments` row.
pub struct NewJudgment<'a> {
    /// The captured query this verdict answers.
    pub observation_id: &'a str,
    /// The candidate that was judged, exactly as it was observed.
    pub candidate_ref: &'a str,
    /// `memory` | `code` | `document`.
    pub domain: &'a str,
    /// Graded relevance, `0..=judgment::MAX_RELEVANCE`.
    pub relevance: i64,
    /// `manual` or `implicit`.
    pub provenance: &'a str,
    /// RFC3339 time the verdict was recorded.
    pub at: &'a str,
}

/// Write every verdict in `rows` as one unit, replacing any previous verdict
/// on the same candidate. Returns how many rows were written.
///
/// `INSERT OR REPLACE` is correct here and is not the row-preserving-upsert
/// case the store forbids: the row has no unassigned fields to reset and
/// nothing references it, so replacing a verdict IS the intended semantics.
pub fn upsert_all(conn: &Connection, rows: &[NewJudgment<'_>]) -> Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }
    let tx = conn.unchecked_transaction()?;
    let mut written = 0_u64;
    for row in rows {
        tx.prepare_cached(
            "INSERT OR REPLACE INTO candidate_judgments\
                 (observation_id, candidate_ref, domain, relevance, provenance, at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(rusqlite::params![
            row.observation_id,
            row.candidate_ref,
            row.domain,
            row.relevance,
            row.provenance,
            row.at,
        ])?;
        written = written.saturating_add(1);
    }
    tx.commit()?;
    Ok(written)
}

/// Every verdict recorded against `observation_id`, keyed by the candidate
/// reference it was made on. An observation with no verdicts yields an empty
/// map, which is what an unjudged observation legitimately is.
pub fn fetch_for_observation(
    conn: &Connection,
    observation_id: &str,
) -> Result<HashMap<String, i64>> {
    let rows = orm::query_all(
        conn,
        CandidateJudgments::select()
            .columns_typed(&[&col::candidate_ref, &col::relevance])
            .filter(col::observation_id.eq(observation_id))
            .to_sql(),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    )?;
    Ok(rows.into_iter().collect())
}

#[cfg(test)]
#[path = "tests/candidate_judgments.rs"]
mod tests;
