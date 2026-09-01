//! `eval_runs` row insert + newest-first read — one row per `comemory
//! eval`/`tune`/`bandit` RUN (never per scored candidate: a `tune` grid
//! scores hundreds of configurations, and only the run's own outcome — for
//! `tune`/`bandit`, the winning candidate — is what the console's run table
//! and recall sparkline show), backing the v14 console-history table
//! (`src/store/sql/0014_v14_console.sql`).

use rusqlite::Connection;
use serde::Serialize;

use crate::prelude::*;

/// Insert parameters for one completed run, bundled into a struct rather
/// than nine positional arguments (`clippy::too_many_arguments`).
pub struct NewRun<'a> {
    /// 16-hex row id (`store::random_id::random_hex(8)`).
    pub id: &'a str,
    /// `"eval"` | `"tune"` | `"bandit"` (matches the table's `CHECK`).
    pub kind: &'a str,
    /// Pre-rendered ISO-8601 UTC timestamp (`store::memory_row::iso_format`).
    pub at: &'a str,
    /// Golden pairs scored.
    pub golden_pairs: u64,
    /// recall@k cut used.
    pub k: u64,
    /// Mean recall@k for this run's reported candidate.
    pub recall: f64,
    /// Mean MRR for this run's reported candidate.
    pub mrr: f64,
    /// Pre-serialized JSON text of the scored knob set.
    pub knobs: &'a str,
    /// Whether this run rewrote `config.toml` (`tune`/`bandit --apply`).
    pub applied: bool,
}

/// One `eval_runs` row, as returned by [`list`].
#[derive(Debug, Serialize)]
pub struct EvalRunRow {
    /// Row id.
    pub id: String,
    /// `"eval"` | `"tune"` | `"bandit"`.
    pub kind: String,
    /// ISO-8601 UTC timestamp this run completed.
    pub at: String,
    /// Golden pairs scored.
    pub golden_pairs: u64,
    /// recall@k cut used.
    pub k: u64,
    /// Mean recall@k for this run's reported candidate.
    pub recall: f64,
    /// Mean MRR for this run's reported candidate.
    pub mrr: f64,
    /// The scored knob set, parsed back into a JSON object (stored as TEXT).
    pub knobs: serde_json::Value,
    /// Whether this run rewrote `config.toml`.
    pub applied: bool,
}

/// Insert one `eval_runs` row. A single `INSERT` with no read-modify-write
/// race — every field is caller-computed (id, timestamp, JSON knobs).
pub fn insert(conn: &Connection, row: &NewRun<'_>) -> Result<()> {
    conn.execute(
        "INSERT INTO eval_runs(id, kind, at, golden_pairs, k, recall, mrr, knobs, applied) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            row.id,
            row.kind,
            row.at,
            i64::try_from(row.golden_pairs).unwrap_or(i64::MAX),
            i64::try_from(row.k).unwrap_or(i64::MAX),
            row.recall,
            row.mrr,
            row.knobs,
            i64::from(row.applied),
        ],
    )?;
    Ok(())
}

/// Read up to `limit` rows, newest-first (`at DESC`, matching
/// `idx_eval_runs_at`). An empty table returns an empty `Vec`, never an
/// error.
pub fn list(conn: &Connection, limit: u32) -> Result<Vec<EvalRunRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, at, golden_pairs, k, recall, mrr, knobs, applied \
         FROM eval_runs ORDER BY at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(rusqlite::params![limit], |r| {
        let golden_pairs: i64 = r.get(3)?;
        let k: i64 = r.get(4)?;
        let knobs_text: String = r.get(7)?;
        let applied: i64 = r.get(8)?;
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            golden_pairs,
            k,
            r.get::<_, f64>(5)?,
            r.get::<_, f64>(6)?,
            knobs_text,
            applied,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, kind, at, golden_pairs, k, recall, mrr, knobs_text, applied) = row?;
        let knobs = serde_json::from_str(&knobs_text)?;
        out.push(EvalRunRow {
            id,
            kind,
            at,
            golden_pairs: golden_pairs.try_into().unwrap_or(0),
            k: k.try_into().unwrap_or(0),
            recall,
            mrr,
            knobs,
            applied: applied != 0,
        });
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests/eval_runs.rs"]
mod tests;
