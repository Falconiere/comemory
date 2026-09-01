//! `api::learning` — the learning-loop console reads: summary, evals with
//! derived flags, golden set, expansions (console-api spec §7).
//!
//! Four independent reads over data the binary already records — the
//! feedback log, `eval_runs`, the golden set, and the mined
//! `query_expansions` table. Nothing here writes; the console's write half
//! is `api::learning_proposals`.
//!
//! **Must-not-create-the-db invariant** (the rule `api::stats` and
//! `api::gc` keep): being asked how the learning loop is doing must not
//! create and migrate a database as a side effect. On a data dir with no
//! `comemory.db`, every function here answers with zeros / an empty page
//! and never calls [`Ctx::conn`].

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::api::Ctx;
use crate::eval::golden::{self, GoldenPair};
use crate::output::page::Page;
use crate::prelude::*;
use crate::store::eval_runs::{self, EvalRunRow};

/// Read every recorded run — the summary's `best_delta` pairs each
/// `tune`/`bandit` row with the nearest EARLIER `eval` row, which can sit
/// arbitrarily far back in the history.
const ALL_RUNS: u32 = u32::MAX;

/// `GET /api/v1/learning/summary` — the Learning screen's header tiles.
#[derive(Serialize, Debug, Default)]
pub struct Summary {
    /// Rows in `feedback_events` (every verdict, both provenances).
    pub feedback_events: u64,
    /// Share of those rows whose `provenance` is not `'manual'` — the
    /// implicit signal the reinforcement loop contributed. `0.0` when no
    /// feedback exists at all (rather than a division by zero).
    pub implicit_share: f64,
    /// `verdict = 'used'` rows.
    pub used: u64,
    /// `verdict = 'irrelevant'` rows.
    pub irrelevant: u64,
    /// The newest `eval_runs` row, whatever its kind.
    pub latest: Option<LatestRun>,
    /// Rows in `query_expansions` (the mined tier-4 ladder's size).
    pub expansions: u64,
    /// The largest recall@k gain any `tune`/`bandit` run showed over the
    /// nearest `eval` run that preceded it. `None` when the history holds
    /// no such pair.
    pub best_delta: Option<f64>,
}

/// The newest recorded run, as [`Summary::latest`] reports it.
#[derive(Serialize, Debug)]
pub struct LatestRun {
    /// `eval_runs` row id.
    pub id: String,
    /// `"eval"` | `"tune"` | `"bandit"`.
    pub kind: String,
    /// ISO-8601 UTC timestamp the run completed.
    pub at: String,
    /// Mean recall@k for the run's reported candidate.
    pub recall_at_k: f64,
    /// Mean MRR for the run's reported candidate.
    pub mrr: f64,
    /// recall@k cut used.
    pub k: u64,
    /// Golden pairs scored.
    pub golden_pairs: u64,
}

/// One row of `GET /api/v1/learning/evals`: the stored run flattened, plus
/// the three fields the console's run table derives rather than stores.
#[derive(Serialize, Debug)]
pub struct EvalRow {
    /// The stored `eval_runs` row, flattened into this object.
    #[serde(flatten)]
    pub run: EvalRunRow,
    /// recall@k minus the chronologically previous returned row's. `None`
    /// on the oldest row of the page — there is nothing to compare it to.
    pub delta: Option<f64>,
    /// Whether this row is a plain `eval` (the measurement a tune/bandit
    /// run is judged against), as opposed to a knob search.
    pub is_baseline: bool,
    /// Whether this row carries the highest recall@k among the returned
    /// rows. Ties flag every row that reaches the maximum.
    pub is_best: bool,
}

/// `GET /api/v1/learning/golden-set` — the effective golden set plus where
/// its pairs came from.
#[derive(Serialize, Debug)]
pub struct GoldenSet {
    /// The merged pairs (a file pair wins over a harvested one on the same
    /// `(query, repo, kind)` key — `golden::merge`'s rule).
    pub pairs: Vec<GoldenPair>,
    /// `pairs.len()`, so a client rendering a count need not walk the list.
    pub count: usize,
    /// Pairs the feedback harvest produced, before the merge.
    pub harvested: usize,
    /// Pairs the YAML file produced, before the merge.
    pub from_file: usize,
}

/// One mined `query_expansions` row, in the console's field names.
#[derive(Serialize, Debug)]
pub struct Expansion {
    /// The failed query term (`query_expansions.term`).
    pub from: String,
    /// The term the successful rewording used (`.expansion`).
    pub to: String,
    /// Observation count backing the mapping (`.support`).
    pub count: u64,
    /// ISO-8601 UTC timestamp of the mining run that wrote the row.
    pub last_mined: String,
}

/// Feedback counters, mined-expansion count, and the run history, in one
/// read. See the module doc for the missing-database rule.
pub fn summary(ctx: &mut Ctx<'_>) -> Result<Summary> {
    if !ctx.paths.db_path().exists() {
        return Ok(Summary::default());
    }
    let conn = ctx.conn()?;
    let (feedback_events, implicit, used, irrelevant) = feedback_counts(conn)?;
    let expansions = count(conn, "SELECT COUNT(*) FROM query_expansions")?;
    let rows = eval_runs::list(conn, ALL_RUNS)?;
    Ok(Summary {
        feedback_events,
        implicit_share: if feedback_events == 0 {
            0.0
        } else {
            implicit as f64 / feedback_events as f64
        },
        used,
        irrelevant,
        latest: rows.first().map(latest_run),
        expansions,
        best_delta: best_delta(&rows),
    })
}

/// Up to `limit` runs, newest-first, each carrying its derived `delta` /
/// `is_baseline` / `is_best`.
pub fn evals(ctx: &mut Ctx<'_>, limit: u32) -> Result<Vec<EvalRow>> {
    if !ctx.paths.db_path().exists() {
        return Ok(Vec::new());
    }
    let conn = ctx.conn()?;
    Ok(derive(eval_runs::list(conn, limit)?))
}

/// The effective golden set: the feedback harvest, merged under an optional
/// YAML file's pairs. Unlike `golden::resolve`, an empty result is a valid
/// answer here (a console screen showing "no pairs yet"), not an error.
///
/// `golden` is a filesystem path the ROUTE has already contained to an
/// allowed root (§Security "Path containment") — this function treats it as
/// already-safe, exactly as `api::eval::run` treats its own `golden`.
pub fn golden_set(ctx: &mut Ctx<'_>, golden: Option<&str>) -> Result<GoldenSet> {
    let file_pairs = match golden {
        Some(p) => golden::load_file(Path::new(p))?,
        None => Vec::new(),
    };
    let harvested = if ctx.paths.db_path().exists() {
        let conn = ctx.conn()?;
        golden::harvest(conn)?
    } else {
        Vec::new()
    };
    let from_file = file_pairs.len();
    let harvested_count = harvested.len();
    let pairs = golden::merge(file_pairs, harvested);
    Ok(GoldenSet {
        count: pairs.len(),
        pairs,
        harvested: harvested_count,
        from_file,
    })
}

/// One page of mined expansions, strongest support first. `term` then
/// `expansion` break the tie — `(term, expansion)` is the table's primary
/// key, so the order is total and a page boundary is stable (two mappings
/// for one term with equal support would otherwise be free to swap between
/// pages); the same order `api::suggest` reads with. `limit == 0` is
/// [`Page`]'s "all" sentinel.
pub fn expansions(ctx: &mut Ctx<'_>, limit: usize, offset: usize) -> Result<Page<Expansion>> {
    if !ctx.paths.db_path().exists() {
        return Ok(Page::new(Vec::new(), limit, offset, Some(0), false));
    }
    let conn = ctx.conn()?;
    let total = count(conn, "SELECT COUNT(*) FROM query_expansions")? as usize;
    let mut stmt = conn.prepare(
        "SELECT term, expansion, support, last_mined FROM query_expansions \
         ORDER BY support DESC, term ASC, expansion ASC LIMIT ?1 OFFSET ?2",
    )?;
    // SQLite reads a negative LIMIT as "no limit", which is exactly what
    // `Page`'s `limit == 0` sentinel means.
    let sql_limit: i64 = if limit == 0 { -1 } else { limit as i64 };
    let items: Vec<Expansion> = stmt
        .query_map(rusqlite::params![sql_limit, offset as i64], |r| {
            Ok(Expansion {
                from: r.get(0)?,
                to: r.get(1)?,
                count: r.get::<_, i64>(2)? as u64,
                last_mined: r.get(3)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    let has_more = offset + items.len() < total;
    Ok(Page::new(items, limit, offset, Some(total), has_more))
}

/// `(total, implicit, used, irrelevant)` over `feedback_events` in one
/// scan. The three conditional sums are `NULL` on an empty table, read
/// back as `0`.
fn feedback_counts(conn: &Connection) -> Result<(u64, u64, u64, u64)> {
    let row = conn.query_row(
        "SELECT COUNT(*), \
                SUM(CASE WHEN provenance != 'manual' THEN 1 ELSE 0 END), \
                SUM(CASE WHEN verdict = 'used' THEN 1 ELSE 0 END), \
                SUM(CASE WHEN verdict = 'irrelevant' THEN 1 ELSE 0 END) \
           FROM feedback_events",
        [],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        },
    )?;
    Ok((
        row.0 as u64,
        row.1.unwrap_or(0) as u64,
        row.2.unwrap_or(0) as u64,
        row.3.unwrap_or(0) as u64,
    ))
}

/// Run a parameterless `COUNT(*)` query.
fn count(conn: &Connection, sql: &str) -> Result<u64> {
    Ok(conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64)
}

/// Project the newest row onto [`LatestRun`].
fn latest_run(row: &EvalRunRow) -> LatestRun {
    LatestRun {
        id: row.id.clone(),
        kind: row.kind.clone(),
        at: row.at.clone(),
        recall_at_k: row.recall,
        mrr: row.mrr,
        k: row.k,
        golden_pairs: row.golden_pairs,
    }
}

/// The biggest recall@k gain of a knob search over the nearest `eval` run
/// that preceded it. Walks `newest_first` in reverse (i.e. chronologically)
/// so "nearest earlier baseline" is just the last `eval` seen.
fn best_delta(newest_first: &[EvalRunRow]) -> Option<f64> {
    let mut baseline: Option<f64> = None;
    let mut best: Option<f64> = None;
    for row in newest_first.iter().rev() {
        match row.kind.as_str() {
            "eval" => baseline = Some(row.recall),
            "tune" | "bandit" => {
                if let Some(base) = baseline {
                    let delta = row.recall - base;
                    best = Some(match best {
                        Some(b) if b.total_cmp(&delta).is_ge() => b,
                        _ => delta,
                    });
                }
            }
            // The table's CHECK admits no fourth kind; a row from a future
            // schema neither sets a baseline nor claims one.
            _ => {}
        }
    }
    best
}

/// Attach `delta` / `is_baseline` / `is_best` to a newest-first run list.
fn derive(rows: Vec<EvalRunRow>) -> Vec<EvalRow> {
    let best = rows
        .iter()
        .map(|r| r.recall)
        .fold(f64::NEG_INFINITY, f64::max);
    let deltas: Vec<Option<f64>> = (0..rows.len())
        .map(|i| rows.get(i + 1).map(|prev| rows[i].recall - prev.recall))
        .collect();
    rows.into_iter()
        .zip(deltas)
        .map(|(run, delta)| EvalRow {
            delta,
            is_baseline: run.kind == "eval",
            is_best: run.recall.total_cmp(&best).is_eq(),
            run,
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/learning.rs"]
mod tests;
