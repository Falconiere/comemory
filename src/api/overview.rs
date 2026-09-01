//! `api::overview` — the console's landing aggregate (`GET /api/v1/overview`,
//! `GET /api/v1/overview/eval-series`), console-api spec §2.
//!
//! Every number here already had a producer; what the Overview screen had
//! no producer for was *one round trip* that returns all of them. This
//! module is that composition and nothing else: it calls
//! [`crate::api::stats::run`] for the counters, [`crate::api::repos::run`]
//! for the per-repo freshness, [`crate::api::list::run`] for the recent
//! memories, and reads `index_runs` / `eval_runs` directly. No counter is
//! recomputed here, so the tiles can never disagree with the screens they
//! link to.
//!
//! **Must-not-create-the-db invariant** (the same rule `api::stats` and
//! `api::repos` keep): a read must not create and migrate a database as a
//! side effect of being asked for a summary. On a data dir with no
//! `comemory.db`, [`run`] never calls [`Ctx::conn`] — it answers with zero
//! counters, an `unknown` index state, and empty lists. The two delegates
//! that guard this themselves are still not enough: `api::list::run` opens
//! a connection unconditionally, so the guard has to be here too.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::api::{Ctx, list, repos, stats};
use crate::prelude::*;
use crate::store::{eval_runs, index_runs, memory_row};

/// How many recent memories the Overview screen's list shows.
const RECENT_MEMORIES: usize = 4;

/// How many eval runs the Overview screen's recall sparkline plots.
pub const EVAL_SERIES_LIMIT: u32 = 20;

/// `GET /api/v1/overview` request.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Scope the per-repo counters, the repo inventory, and the recent
    /// memory list to one repo label. The database size, the edge count,
    /// and the eval history stay global — see [`stats::Request::repo`].
    #[serde(default)]
    pub repo: Option<String>,
}

/// The corpus tiles, projected from [`stats::Response`]. `graph_edges`
/// renames that response's `edges` for the console's tile label; every
/// other field is the same number under the same name.
#[derive(Serialize, Debug, Default)]
pub struct Counters {
    /// Live memory rows.
    pub memories: u64,
    /// Rows in `code_symbols`, cAST chunk children included.
    pub code_symbols: u64,
    /// Rows in `edges`, every relation kind together.
    pub graph_edges: u64,
    /// Logical database size (`page_count * page_size`).
    pub db_bytes: u64,
    /// Rows in `documents`.
    pub documents: u64,
    /// Soft-deleted memory rows still in the mirror.
    pub trashed: u64,
}

/// One repo's index freshness, the per-row half of [`IndexState`].
#[derive(Serialize, Debug)]
pub struct RepoState {
    /// The repo label.
    pub repo: String,
    /// `"fresh"` | `"stale"` | `"unknown"`, verbatim from [`repos::Row`].
    pub status: String,
    /// Files changed since the last index, when the repo is stale and git
    /// could answer.
    pub changed_files: Option<u64>,
}

/// Whether the code index is caught up with the working trees on disk.
#[derive(Serialize, Debug)]
pub struct IndexState {
    /// The worst status across every indexed repo: `"stale"` if any repo
    /// is stale, else `"fresh"` if any is fresh, else `"unknown"`. A single
    /// stale repo makes the whole index stale — the banner exists to say
    /// "something needs reindexing", and averaging that away would hide it.
    pub status: String,
    /// Changed files summed over the stale repos only. A fresh repo has
    /// nothing to count and an `unknown` one could not be asked.
    pub changed_files: u64,
    /// The `last_head` of the most recently indexed repo, when one exists.
    pub indexed_commit: Option<String>,
    /// When this freshness check ran (now — the git probes behind it are
    /// live, not cached).
    pub checked_at: String,
    /// Per-repo detail behind the rolled-up status.
    pub repos: Vec<RepoState>,
}

/// The `co_changed` / `imports` edge totals the last-run tile shows beside
/// the file and symbol counts. Global counts, not per-run: the graph is
/// materialized wholesale, so there is no per-run delta to attribute.
#[derive(Serialize, Debug, Default)]
pub struct EdgeCounts {
    /// Rows in `edges` with `rel = 'co_changed'`.
    pub cochange: u64,
    /// Rows in `edges` with `rel = 'imports'`.
    pub imports: u64,
}

/// The newest `index_runs` row, plus the code-graph edge totals.
#[derive(Serialize, Debug)]
pub struct LastRun {
    /// Row id.
    pub id: String,
    /// The repo label the run indexed.
    pub repo: String,
    /// `"full"` | `"incremental"`.
    pub mode: String,
    /// ISO-8601 UTC start timestamp.
    pub started_at: String,
    /// ISO-8601 UTC finish timestamp.
    pub finished_at: String,
    /// Wall-clock duration of the run.
    pub duration_ms: u64,
    /// Files actually (re)indexed by this run.
    pub files_indexed: u64,
    /// `code_symbols` rows for the repo after the run.
    pub symbols: u64,
    /// `"ok"` | `"error"` | `"cancelled"`.
    pub outcome: String,
    /// The failure message for an `error` outcome, else `None`.
    pub error: Option<String>,
    /// Current code-graph edge totals.
    pub edges: EdgeCounts,
}

/// The newest eval run's headline numbers.
#[derive(Serialize, Debug)]
pub struct Metrics {
    /// Mean recall@k for that run's reported candidate.
    pub recall_at_k: f64,
    /// Mean MRR for that run's reported candidate.
    pub mrr: f64,
    /// The recall@k cut used.
    pub k: u64,
    /// Golden pairs scored.
    pub golden_queries: u64,
    /// ISO-8601 UTC timestamp the run completed.
    pub at: String,
}

/// One point on the Overview screen's recall sparkline.
#[derive(Serialize, Debug)]
pub struct EvalPoint {
    /// `eval_runs` row id.
    pub id: String,
    /// ISO-8601 UTC timestamp the run completed.
    pub at: String,
    /// `"eval"` | `"tune"` | `"bandit"`.
    pub kind: String,
    /// Mean recall@k.
    pub recall_at_k: f64,
    /// Mean MRR.
    pub mrr: f64,
}

/// `GET /api/v1/overview` response.
#[derive(Serialize)]
pub struct Response {
    /// Corpus tiles.
    pub counters: Counters,
    /// Code-index freshness.
    pub index_state: IndexState,
    /// The newest index run, or `None` before any run.
    pub last_run: Option<LastRun>,
    /// The newest eval run's headline numbers, or `None` before any run.
    pub metrics: Option<Metrics>,
    /// The recall sparkline, OLDEST first (a chart reads left to right,
    /// while [`eval_runs::list`] answers newest first).
    pub eval_series: Vec<EvalPoint>,
    /// The newest memories, at most [`RECENT_MEMORIES`] of them.
    pub recent_memories: Vec<list::Row>,
}

/// Collect the whole landing aggregate. See the module doc for why a
/// missing database is reported rather than created.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let checked_at = memory_row::iso_format(OffsetDateTime::now_utc())?;
    if !ctx.paths.db_path().exists() {
        return Ok(empty(checked_at));
    }
    let counters = counters_of(stats::run(
        ctx,
        stats::Request {
            repo: req.repo.clone(),
        },
    )?);
    let inventory = repos::run(
        ctx,
        repos::Request {
            repo: req.repo.clone(),
        },
    )?;
    let index_state = index_state_of(inventory.repos, checked_at);
    let recent_memories = list::run(ctx, recent_request(req.repo))?.items;

    let conn = ctx.conn()?;
    let last_run = last_run_of(conn)?;
    // ONE read backs both the sparkline and the headline metrics: the
    // newest of these rows IS `eval_runs::list(conn, 1)`'s only row, so a
    // second query would just be the same statement asked twice.
    let runs = eval_runs::list(conn, EVAL_SERIES_LIMIT)?;
    let metrics = runs.first().map(metrics_of);
    Ok(Response {
        counters,
        index_state,
        last_run,
        metrics,
        eval_series: series_of(runs),
        recent_memories,
    })
}

/// `GET /api/v1/overview/eval-series?limit=` — the sparkline on its own, so
/// the console can refresh it after an eval without re-fetching every tile.
/// Oldest first, exactly as [`Response::eval_series`]. Keeps [`run`]'s
/// must-not-create-the-db guard.
pub fn eval_series(ctx: &mut Ctx<'_>, limit: u32) -> Result<Vec<EvalPoint>> {
    if !ctx.paths.db_path().exists() {
        return Ok(Vec::new());
    }
    let runs = eval_runs::list(ctx.conn()?, limit)?;
    Ok(series_of(runs))
}

/// The all-zero answer for a data dir with no database yet.
fn empty(checked_at: String) -> Response {
    Response {
        counters: Counters::default(),
        index_state: IndexState {
            status: UNKNOWN.to_string(),
            changed_files: 0,
            indexed_commit: None,
            checked_at,
            repos: Vec::new(),
        },
        last_run: None,
        metrics: None,
        eval_series: Vec::new(),
        recent_memories: Vec::new(),
    }
}

/// The `repos`/`index_state` status vocabulary, shared with
/// `api::repos::git_state` (which produces the per-row values this rolls up).
const STALE: &str = "stale";
const FRESH: &str = "fresh";
const UNKNOWN: &str = "unknown";

/// Project the corpus counters onto the console's tile names.
fn counters_of(s: stats::Response) -> Counters {
    Counters {
        memories: s.memories,
        code_symbols: s.code_symbols,
        graph_edges: s.edges,
        db_bytes: s.db_bytes,
        documents: s.documents,
        trashed: s.trashed,
    }
}

/// The `list` request behind `recent_memories`: the newest few, default
/// sort, scoped to the same repo as the counters.
fn recent_request(repo: Option<String>) -> list::Request {
    list::Request {
        repo,
        kind: None,
        tag: None,
        min_quality: None,
        q: None,
        limit: RECENT_MEMORIES,
        offset: 0,
        sort: list::Sort::default(),
    }
}

/// Roll the per-repo inventory up into one index state (see
/// [`IndexState::status`] for why the worst status wins).
fn index_state_of(rows: Vec<repos::Row>, checked_at: String) -> IndexState {
    let status = if rows.iter().any(|r| r.status == STALE) {
        STALE
    } else if rows.iter().any(|r| r.status == FRESH) {
        FRESH
    } else {
        UNKNOWN
    };
    let changed_files = rows
        .iter()
        .filter(|r| r.status == STALE)
        .map(|r| r.changed_files.unwrap_or(0))
        .sum();
    let indexed_commit = rows
        .iter()
        .filter(|r| r.last_head.is_some())
        .max_by(|a, b| a.last_indexed_at.cmp(&b.last_indexed_at))
        .and_then(|r| r.last_head.clone());
    IndexState {
        status: status.to_string(),
        changed_files,
        indexed_commit,
        checked_at,
        repos: rows
            .into_iter()
            .map(|r| RepoState {
                repo: r.repo,
                status: r.status,
                changed_files: r.changed_files,
            })
            .collect(),
    }
}

/// The newest `index_runs` row plus the current code-graph edge totals.
fn last_run_of(conn: &Connection) -> Result<Option<LastRun>> {
    let Some(row) = index_runs::newest(conn)? else {
        return Ok(None);
    };
    Ok(Some(LastRun {
        id: row.id,
        repo: row.repo,
        mode: row.mode,
        started_at: row.started_at,
        finished_at: row.finished_at,
        duration_ms: row.duration_ms,
        files_indexed: row.files_indexed,
        symbols: row.symbols,
        outcome: row.outcome,
        error: row.error,
        edges: EdgeCounts {
            cochange: edge_count(conn, "co_changed")?,
            imports: edge_count(conn, "imports")?,
        },
    }))
}

/// `COUNT(*)` over one `edges` relation kind. `rel` is bound as a
/// parameter, never interpolated.
fn edge_count(conn: &Connection, rel: &str) -> Result<u64> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM edges WHERE rel = ?1", [rel], |r| {
        r.get(0)
    })?;
    Ok(u64::try_from(n).unwrap_or(0))
}

/// The headline numbers of one eval run.
fn metrics_of(row: &eval_runs::EvalRunRow) -> Metrics {
    Metrics {
        recall_at_k: row.recall,
        mrr: row.mrr,
        k: row.k,
        golden_queries: row.golden_pairs,
        at: row.at.clone(),
    }
}

/// Turn a newest-first run list into an oldest-first sparkline.
fn series_of(runs: Vec<eval_runs::EvalRunRow>) -> Vec<EvalPoint> {
    runs.into_iter()
        .rev()
        .map(|r| EvalPoint {
            id: r.id,
            at: r.at,
            kind: r.kind,
            recall_at_k: r.recall,
            mrr: r.mrr,
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/overview.rs"]
mod tests;
