//! `eval_runs` row insert + newest-first read — one row per `comemory
//! eval`/`tune`/`bandit` RUN (never per scored candidate: a `tune` grid
//! scores hundreds of configurations, and only the run's own outcome — for
//! `tune`/`bandit`, the winning candidate — is what the console's run table
//! and recall sparkline show), backing the v14 console-history table
//! (`src/store/sql/0014_v14_console.sql`).
//!
//! v15 adds the two flag writers the console's knob proposals need:
//! [`set_applied`] (the run's knobs were written into `config.toml`) and
//! [`set_discarded`] (the run was dismissed without applying), plus
//! [`get`] for the single-row read those routes do first.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::prelude::*;

/// The column list every reader selects, in the order [`read_row`] expects.
///
/// A macro rather than a `const` so it can be spliced with [`concat!`],
/// which takes literals only: both SELECT statements below are then compile-time
/// `&'static str`s with no runtime string building, and the list still has
/// exactly one definition.
macro_rules! columns {
    () => {
        "id, kind, at, golden_pairs, k, recall, mrr, knobs, applied, discarded"
    };
}

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
    /// Whether this run's knobs were dismissed as a proposal (v15's
    /// `eval_runs.discarded`) — the one proposal state that cannot be
    /// derived from anything else already stored.
    pub discarded: bool,
}

/// One row exactly as SQLite hands it over: `knobs` still TEXT, the two
/// flags still integers. Split from [`EvalRunRow`] because the JSON parse
/// of `knobs` can fail with a crate `Error`, which a `rusqlite` row mapper
/// cannot return — [`finish`] does that half.
struct RawRow {
    id: String,
    kind: String,
    at: String,
    golden_pairs: i64,
    k: i64,
    recall: f64,
    mrr: f64,
    knobs: String,
    applied: i64,
    discarded: i64,
}

/// Map one [`columns!`]-shaped result row into a [`RawRow`]. Shared by
/// [`list`] and [`get`] so the two readers cannot drift on column order.
fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        at: r.get(2)?,
        golden_pairs: r.get(3)?,
        k: r.get(4)?,
        recall: r.get(5)?,
        mrr: r.get(6)?,
        knobs: r.get(7)?,
        applied: r.get(8)?,
        discarded: r.get(9)?,
    })
}

/// Parse a [`RawRow`]'s `knobs` TEXT into JSON and widen its counters.
fn finish(raw: RawRow) -> Result<EvalRunRow> {
    Ok(EvalRunRow {
        id: raw.id,
        kind: raw.kind,
        at: raw.at,
        golden_pairs: raw.golden_pairs.try_into().unwrap_or(0),
        k: raw.k.try_into().unwrap_or(0),
        recall: raw.recall,
        mrr: raw.mrr,
        knobs: serde_json::from_str(&raw.knobs)?,
        applied: raw.applied != 0,
        discarded: raw.discarded != 0,
    })
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
    let mut stmt = conn.prepare(concat!(
        "SELECT ",
        columns!(),
        " FROM eval_runs ORDER BY at DESC LIMIT ?1"
    ))?;
    let rows = stmt.query_map(rusqlite::params![limit], read_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(finish(row?)?);
    }
    Ok(out)
}

/// Read one row by id. `Ok(None)` when no such run exists — an unknown id
/// is a caller-visible outcome (the console's proposal routes turn it into
/// a `404`), not an error this layer decides.
pub fn get(conn: &Connection, id: &str) -> Result<Option<EvalRunRow>> {
    let raw = conn
        .query_row(
            concat!("SELECT ", columns!(), " FROM eval_runs WHERE id = ?1"),
            rusqlite::params![id],
            read_row,
        )
        .optional()?;
    raw.map(finish).transpose()
}

/// Which proposal flag [`set_flag`] stamps. An enum rather than a column
/// name: each variant maps to one whole literal `UPDATE`, so there is no
/// column identifier to build at runtime and no caller-supplied string that
/// could reach the SQL, whatever its type.
enum Flag {
    /// `applied = 1`.
    Applied,
    /// `discarded = 1`.
    Discarded,
}

impl Flag {
    /// The complete statement for this flag.
    const fn sql(&self) -> &'static str {
        match self {
            Self::Applied => "UPDATE eval_runs SET applied = 1 WHERE id = ?1",
            Self::Discarded => "UPDATE eval_runs SET discarded = 1 WHERE id = ?1",
        }
    }
}

/// Set one flag on one row, erroring with [`Error::NotFound`] when the id
/// matched nothing — a silent zero-row `UPDATE` would report success for a
/// run that does not exist.
fn set_flag(conn: &Connection, id: &str, flag: &Flag) -> Result<()> {
    let changed = conn.execute(flag.sql(), rusqlite::params![id])?;
    if changed == 0 {
        return Err(Error::NotFound(format!("eval run {id}")));
    }
    Ok(())
}

/// Stamp a run as applied (`applied = 1`) — the console's
/// `POST /learning/proposals/{id}/apply` after it rewrites `config.toml`.
pub fn set_applied(conn: &Connection, id: &str) -> Result<()> {
    set_flag(conn, id, &Flag::Applied)
}

/// Stamp a run as discarded (`discarded = 1`) — the console's
/// `POST /learning/proposals/{id}/discard`, which touches no config file.
pub fn set_discarded(conn: &Connection, id: &str) -> Result<()> {
    set_flag(conn, id, &Flag::Discarded)
}

#[cfg(test)]
#[path = "tests/eval_runs.rs"]
mod tests;
