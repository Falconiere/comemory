//! FTS5 memory leg: the four lexical ladder tiers and the single MATCH
//! executor behind them.
//!
//! Split out of [`crate::store::fts`], which keeps the shared query
//! builders ([`build_match_query`] and friends), the code leg
//! (`search_code`), and the FTS5 parse-error plumbing
//! ([`run_fts_query`]). Every tier funnels through [`run_memory_match`],
//! so the memory leg has exactly one place where its SQL, filters, and
//! BM25 weights live. `fts` re-exports this module's items, so callers
//! keep using `fts::search_memory*` paths.

use rusqlite::{Connection, params};

use crate::prelude::*;
use crate::store::CreatedWindow;
use crate::store::fts::{
    build_expanded_or_query, build_match_query, build_or_query, build_subtoken_or_query,
    run_fts_query,
};

/// FTS5 hit for the memory table; lower `score` (BM25) = better match.
pub struct MemoryFtsHit {
    /// Identifier of the matched memory row.
    pub memory_id: String,
    /// BM25 relevance score; lower is better.
    pub score: f32,
}

/// The four ladder tiers share one signature and one MATCH executor; only
/// the MATCH-expression builder varies. A table keeps them from pairing
/// while each name still encodes its tier.
macro_rules! memory_search_tiers {
    ($(
        $(#[$meta:meta])*
        $name:ident => $build:expr
    ),+ $(,)?) => {
        $(
            $(#[$meta])*
            pub fn $name(
                conn: &Connection,
                query: &str,
                k: usize,
                repo: Option<&str>,
                kind: Option<&str>,
                window: CreatedWindow<'_>,
                weights: (f32, f32),
            ) -> Result<Vec<MemoryFtsHit>> {
                let match_expr = ($build)(conn, query)?;
                run_memory_match(conn, &match_expr, k, repo, kind, window, weights)
            }
        )+
    };
}

memory_search_tiers! {
    /// Run a BM25 search over `memory_fts`, skipping soft-deleted memories.
    ///
    /// The query is rewritten via [`build_match_query`] (quoted terms, last
    /// term prefix-matched) and ranked with a weighted BM25 whose
    /// `(body, tags)` weights come from `weights`
    /// (`cfg.retrieval.bm25_weights`). Optional `repo` / `kind` filters ride
    /// the same JOIN that gates on `deleted_at`, so the lexical and vector
    /// branches share one scope on a filtered hybrid query (`kind` is the
    /// lowercase string stored in `memories.kind`, e.g. `decision`), as does
    /// the `window` created-date bound. FTS5 MATCH parse errors are downgraded
    /// to an empty result so a typo cannot abort the wider pipeline.
    search_memory => |_conn: &Connection, q: &str| -> Result<String> {
        Ok(build_match_query(q))
    },
    /// Relaxed variant of [`search_memory`]: OR-joins the sanitized terms via
    /// [`build_or_query`] so a memory matching any single term still surfaces.
    /// Used by the router as a fallback tier when the strict query is empty.
    search_memory_relaxed => |_conn: &Connection, q: &str| -> Result<String> {
        Ok(build_or_query(q))
    },
    /// Subtoken variant of [`search_memory`]: OR-joins the identifier
    /// sub-tokens of every sanitized term via [`build_subtoken_or_query`] so a
    /// memory whose prose mentions the *parts* of an identifier (`dim
    /// mismatch` for `VecDimMismatch`) still surfaces. Used by the router as
    /// the final fallback tier when both the strict AND and the word-level OR
    /// tiers are empty. A query with no splittable term builds an empty MATCH
    /// expression and returns an empty result.
    search_memory_subtokens => |_conn: &Connection, q: &str| -> Result<String> {
        Ok(build_subtoken_or_query(q))
    },
    /// Tier-4 search: learned-expansion OR query via
    /// [`build_expanded_or_query`], so a memory containing only a mined
    /// expansion of a query term still surfaces. An empty expression (no
    /// applicable expansion) returns empty without a useful MATCH.
    search_memory_expanded => |conn: &Connection, q: &str| -> Result<String> {
        build_expanded_or_query(conn, q)
    },
}

/// Execute a prebuilt MATCH expression against `memory_fts`. Shared by
/// every tier above so the strict, relaxed, subtoken, and expanded tiers
/// cannot drift on SQL, weights, or error handling.
///
/// `weights` follows the `memory_fts` column order
/// `(memory_id UNINDEXED, body, tags)` with the UNINDEXED column pinned
/// to 0; with the default `(1.0, 3.0)` a tag hit outranks a body hit.
/// The weights are interpolated into the SQL text rather than bound:
/// `bm25()` arguments cannot be parameters, and the values come from
/// validated config (finite, >= 0), never raw user input. FTS5 `bm25()`
/// returns negative scores (more negative = better), so `ORDER BY score`
/// ascending keeps best-first. `window` bounds `memories.created_at`
/// through `datetime()`, immune to mixed stored precision.
fn run_memory_match(
    conn: &Connection,
    match_expr: &str,
    k: usize,
    repo: Option<&str>,
    kind: Option<&str>,
    window: CreatedWindow<'_>,
    weights: (f32, f32),
) -> Result<Vec<MemoryFtsHit>> {
    if match_expr.is_empty() || k == 0 {
        return Ok(Vec::new());
    }
    // `?3 IS NULL OR m.repo = ?3` (and `?4` for kind, `?5`/`?6` for the
    // created-date window) lets us bind each optional filter as a single
    // SQL string. SQLite short-circuits on the first disjunct when the
    // parameter is NULL, so an absent filter is a no-op. A row whose
    // `created_at` does not parse yields NULL from `datetime()` and drops
    // out of any bounded window — unbounded runs still see it.
    let (w_body, w_tags) = weights;
    let sql = format!(
        "SELECT memory_fts.memory_id, bm25(memory_fts, 0.0, {w_body}, {w_tags}) AS score \
           FROM memory_fts \
           JOIN memories m ON m.id = memory_fts.memory_id \
          WHERE memory_fts MATCH ?1 AND m.deleted_at IS NULL \
            AND (?3 IS NULL OR m.repo = ?3) \
            AND (?4 IS NULL OR m.kind = ?4) \
            AND (?5 IS NULL OR datetime(m.created_at) >= datetime(?5)) \
            AND (?6 IS NULL OR datetime(m.created_at) <= datetime(?6)) \
          ORDER BY score \
          LIMIT ?2"
    );
    run_fts_query(
        conn,
        &sql,
        params![
            match_expr,
            k as i64,
            repo,
            kind,
            window.since,
            window.cutoff
        ],
        |row| {
            Ok(MemoryFtsHit {
                memory_id: row.get(0)?,
                score: row.get(1)?,
            })
        },
    )
}

#[cfg(test)]
#[path = "tests/fts_memory.rs"]
mod tests;
