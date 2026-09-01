//! `api::graph_recompute` — `POST /api/v1/graph/recompute`: re-run PageRank
//! and re-project it for every repo, then the memory rank (console-api
//! spec §5).
//!
//! Recompute, not re-index: the `imports` / `co_changed` edges are read
//! exactly as stored — no git history is mined and no source file is
//! re-walked — so this is the cheap way to restore `code_symbols.rank_score`
//! and `memories.rank_score` after rows were edited outside the indexer (a
//! `rebuild`, a repo disconnect, a hand-repaired database). Re-indexing a
//! repo is `POST /index/runs`.
//!
//! The code side runs [`materialize::recompute_rank`] — the very tail of
//! `index-code`'s graph post-pass — once per repo inside ONE transaction, so
//! a mid-run failure leaves every score at its previous value rather than
//! half-updated. The memory side then goes through
//! [`derived::refresh_derived_best_effort`], the single seam that refreshes
//! `memories.rank_score` AND the `edge_fts` triplet index together; both
//! halves are best-effort there by design, and a failure of either is
//! logged rather than failing the job, exactly as at every other write seam.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::graph::{derived, materialize};
use crate::prelude::*;

/// `POST /api/v1/graph/recompute` request. The recompute always covers
/// every indexed repo — a per-repo variant would leave PageRank comparable
/// only within a repo, which it already is — so there is nothing to carry.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Request {}

/// `POST /api/v1/graph/recompute` result, reported as the job's `result`.
#[derive(Serialize, Debug)]
pub struct Response {
    /// The repo labels that were rescored, ascending.
    pub repos: Vec<String>,
    /// `code_symbols` rows whose `rank_score` was written, across all repos.
    pub symbols_scored: u64,
    /// Live memories carrying a refreshed `memories.rank_score`.
    pub memories_scored: u64,
}

/// Rescore every indexed repo's code graph, then the memory graph.
pub fn run(ctx: &mut Ctx<'_>, _req: Request) -> Result<Response> {
    let conn = ctx.conn()?;
    let repos = known_repos(conn)?;
    let symbols_scored = recompute_repos(conn, &repos)?;
    derived::refresh_derived_best_effort(conn);
    let memories_scored = live_memory_count(conn)?;
    Ok(Response {
        repos,
        symbols_scored,
        memories_scored,
    })
}

/// Every repo the store knows about, ascending. `repo_marker` is the
/// registry `index-code` stamps for each repo it walks, so it is the
/// authoritative repo list — a `code_symbols` scan would answer the same
/// thing more expensively.
fn known_repos(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT repo FROM repo_marker ORDER BY repo")?;
    let rows = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(rows)
}

/// Rescore each repo in one shared transaction, returning the total number
/// of `code_symbols` rows written. A repo with no indexed files contributes
/// `0` rather than failing — an archived or freshly registered repo is not
/// an error.
fn recompute_repos(conn: &mut Connection, repos: &[String]) -> Result<u64> {
    let tx = conn.transaction()?;
    let mut written: u64 = 0;
    for repo in repos {
        written = written.saturating_add(materialize::recompute_rank(&tx, repo)?);
    }
    tx.commit()?;
    Ok(written)
}

/// Count of live memories — the number of `memories.rank_score` values
/// [`derived::refresh_derived_best_effort`] just wrote, since the memory
/// PageRank scores every live row and no other.
fn live_memory_count(conn: &Connection) -> Result<u64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE deleted_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    Ok(u64::try_from(count).unwrap_or(0))
}

#[cfg(test)]
#[path = "tests/graph_recompute.rs"]
mod tests;
