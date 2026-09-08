//! Per-symbol ranking signals: the `code_symbols` + `code_feedback` join
//! behind [`crate::retrieval::code_prior`]'s four-prior scorer.
//!
//! Moved out of `retrieval::code_prior` (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`): the SQL
//! text and row mapping live here, unchanged; the prior math itself
//! (`code_prior::priors`, `median_file_rank`, the affinity cache) stays in
//! `retrieval`, which re-exports [`signals`], [`signals_batch`] and
//! [`Signals`] so its own callers (`bundle`, `code_rerank`) are unaffected.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};

use crate::prelude::*;

/// Per-symbol ranking signals pulled in one query: identity columns,
/// rank/access counters, and the (optional) feedback counters with
/// `COALESCE` neutralizing absent rows. `Clone` so [`signals_batch`]'s
/// caller can map one fetched row onto several refs that share a `symbol_id`
/// (the same symbol cited by two memories), matching the per-ref `signals`
/// fetch's behavior of returning `Some` for every duplicate.
#[derive(Clone)]
pub struct Signals {
    /// Repository the symbol was indexed from.
    pub repo: String,
    /// Repo-relative file path.
    pub path: String,
    /// Qualified symbol name.
    pub symbol: String,
    /// Symbol kind, e.g. `function`.
    pub kind: String,
    /// Source language, e.g. `rust`.
    pub lang: String,
    /// First line of the symbol.
    pub line_start: i64,
    /// Last line of the symbol.
    pub line_end: i64,
    /// Projected PageRank score of the symbol's file.
    pub rank_score: f64,
    /// Times the symbol was returned by a tracked search.
    pub access_count: u64,
    /// Last access timestamp (falls back to `indexed_at`).
    pub last_accessed: String,
    /// Parent `code_symbols` rowid for cAST chunk rows.
    pub parent_id: Option<i64>,
    /// `code_feedback.used_count` under the row's effective identity.
    pub used: u64,
    /// `code_feedback.irrelevant_count` under the row's effective identity.
    pub irrelevant: u64,
}

/// Column projection + `code_feedback` join shared by [`signals`] and
/// [`signals_batch`]. Prefixing with `c.id` lets the batch map rows back by
/// id while the single-row form ignores column 0. `code_feedback` is keyed by
/// stable (repo, path, symbol) identity (see `stats::code_feedback`), joined
/// by the row's EFFECTIVE identity: the CLI feedback path records against the
/// COALESCED parent id, so a cAST chunk row (`parent_id` NOT NULL, symbol
/// `<name>#<n>`) never owns a feedback row of its own — it inherits the
/// PARENT's counters via the `COALESCE(parent.symbol, c.symbol)` join so the
/// parent's feedback influences its chunks while they are scored pre-coalesce.
const SIGNALS_SELECT: &str =
    "SELECT c.id, c.repo, c.path, c.symbol, c.kind, c.lang, c.line_start, c.line_end,
            c.rank_score, c.access_count, COALESCE(c.last_accessed, c.indexed_at),
            c.parent_id, COALESCE(f.used_count, 0), COALESCE(f.irrelevant_count, 0)
       FROM code_symbols c
       LEFT JOIN code_feedback f
              ON f.repo = c.repo AND f.path = c.path
             AND f.symbol = COALESCE(
                   (SELECT p.symbol FROM code_symbols p WHERE p.id = c.parent_id),
                   c.symbol)";

/// Map one [`SIGNALS_SELECT`] row into `(id, Signals)`. Column 0 is the
/// `code_symbols` rowid; the remaining columns are the signal fields.
fn map_signals(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, Signals)> {
    Ok((
        r.get(0)?,
        Signals {
            repo: r.get(1)?,
            path: r.get(2)?,
            symbol: r.get(3)?,
            kind: r.get(4)?,
            lang: r.get(5)?,
            line_start: r.get(6)?,
            line_end: r.get(7)?,
            rank_score: r.get(8)?,
            access_count: r.get::<_, i64>(9)?.max(0) as u64,
            last_accessed: r.get(10)?,
            parent_id: r.get(11)?,
            used: r.get::<_, i64>(12)?.max(0) as u64,
            irrelevant: r.get::<_, i64>(13)?.max(0) as u64,
        },
    ))
}

/// Fetch the ranking signals for one code symbol. Returns `Ok(None)` when
/// the row vanished (raced re-index delete). `prepare_cached` so per-hit
/// loops reuse one prepared statement. Shares [`SIGNALS_SELECT`] with
/// [`signals_batch`] so the two cannot drift.
pub fn signals(conn: &Connection, symbol_id: i64) -> Result<Option<Signals>> {
    let sql = format!("{SIGNALS_SELECT} WHERE c.id = ?1");
    let mut stmt = conn.prepare_cached(&sql)?;
    stmt.query_row([symbol_id], map_signals)
        .map(|(_, sig)| sig)
        .optional()
        .map_err(Error::from)
}

/// Max ids per batched [`signals_batch`] chunk — one host param each, well
/// under bundled SQLite's `SQLITE_MAX_VARIABLE_NUMBER` (32766).
const SIGNALS_ID_CHUNK: usize = 500;

/// Fetch the ranking signals for many symbols in one chunked query, keyed by
/// `code_symbols.id`. Runs the SAME [`SIGNALS_SELECT`] (identical projection,
/// feedback join, and parent-symbol subquery) as [`signals`], so a row fetched
/// in a batch is byte-identical to the same row fetched one-at-a-time. Ids that
/// vanished (raced re-index delete) simply have no map entry — the caller
/// treats a miss as the same `None` the single-row form returns. Duplicate ids
/// in `ids` collapse to one entry. `prepare_cached` keys one statement per
/// chunk arity.
pub fn signals_batch(conn: &Connection, ids: &[i64]) -> Result<HashMap<i64, Signals>> {
    let mut out: HashMap<i64, Signals> = HashMap::with_capacity(ids.len());
    for chunk in ids.chunks(SIGNALS_ID_CHUNK) {
        if chunk.is_empty() {
            continue;
        }
        let marks = crate::store::qmarks(chunk.len());
        let sql = format!("{SIGNALS_SELECT} WHERE c.id IN ({marks})");
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), map_signals)?;
        for row in rows {
            let (id, sig) = row?;
            out.insert(id, sig);
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests/code_signals.rs"]
mod tests;
