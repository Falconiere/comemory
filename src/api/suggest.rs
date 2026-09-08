//! `api::suggest` — `GET /api/v1/search/suggest`: mined expansions and recent
//! queries for the ⌘K palette (console-api spec §3).
//!
//! Two independent lists, both read-only and both derived from the learning
//! loop's own tables — nothing new is mined, logged, or ranked here:
//!
//! - **expansions**: rows of `query_expansions` whose `term` is one of the
//!   typed query's tokens, strongest support first. These are the terms
//!   `retrieval::router`'s tier-4 ladder would itself reach for, so
//!   offering them as completions shows the user the vocabulary the index
//!   already knows.
//! - **recent**: distinct `retrieval_log` queries that start with what the
//!   user typed, newest first. `search-code` rows are excluded for the same
//!   reason mining excludes them (`stats::source::SEARCH_CODE`): they are a
//!   different query vocabulary and can only ever earn code feedback.

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::prelude::*;
use crate::stats::source::SEARCH_CODE;
use crate::store::memory_list::like_escape;
use crate::store::tokenizer::split::query_tokens;
use crate::store::{Connection, query_expansions, retrieval_log};

/// Rows returned per list when the request omits `limit`.
const DEFAULT_LIMIT: usize = 10;

/// `GET /api/v1/search/suggest` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// What the user has typed so far. Empty (or blank) is a `400`: an
    /// empty prefix would ask for the whole query log, which is a different
    /// endpoint's job.
    pub q: String,
    /// Maximum rows per list (default [`DEFAULT_LIMIT`]). Applied to each
    /// list independently, not to their sum.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// One mined query expansion.
#[derive(Serialize, Debug)]
pub struct Expansion {
    /// The token from the typed query that matched.
    pub term: String,
    /// The term the mining pass learned to add for it.
    pub expansion: String,
    /// How many reformulation pairs support the mapping.
    pub support: u64,
}

/// One previously-run query.
#[derive(Serialize, Debug)]
pub struct RecentQuery {
    /// The query text as it was run.
    pub query: String,
    /// The `retrieval_log` id of the newest run of that text — the id a
    /// caller would pass to `POST /feedback`.
    pub query_id: String,
    /// ISO-8601 UTC timestamp of that run.
    pub at: String,
}

/// `GET /api/v1/search/suggest` response.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Mined expansions for the typed query's tokens, strongest first.
    pub expansions: Vec<Expansion>,
    /// Recent distinct queries sharing the typed prefix, newest first.
    pub recent: Vec<RecentQuery>,
}

/// Collect both suggestion lists for `req.q`.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let q = req.q.trim().to_string();
    if q.is_empty() {
        return Err(Error::BadRequest("q must not be empty".into()));
    }
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT);
    let conn = ctx.conn()?;
    Ok(Response {
        expansions: expansions(conn, &q, limit)?,
        recent: recent(conn, &q, limit)?,
    })
}

/// Mined expansions for every token of `q`, ordered by descending support
/// (ties broken on `term`/`expansion` so the list is deterministic).
///
/// The token set comes from the SAME splitter the FTS5 tokenizer and the
/// mining pass use, so a suggestion is offered exactly when the ladder
/// would have used it. No fallback tier on purpose: an empty expansion
/// list is the honest answer for a query the mining pass has never seen.
fn expansions(conn: &Connection, q: &str, limit: usize) -> Result<Vec<Expansion>> {
    let terms: Vec<String> = query_tokens(q).into_iter().collect();
    let rows = query_expansions::matching_terms(conn, &terms, limit)?;
    Ok(rows
        .into_iter()
        .map(|r| Expansion {
            term: r.term,
            expansion: r.expansion,
            support: u64::try_from(r.support).unwrap_or(0),
        })
        .collect())
}

/// Distinct past queries starting with `q`, newest first.
///
/// SQLite's `LIKE` is ASCII-case-insensitive by default, which is exactly
/// the matching a completion palette wants. Deduplication is done in Rust
/// rather than with `GROUP BY`: the newest row of each distinct text is
/// wanted *with its own* `query_id`, and a bare `GROUP BY` would pick an
/// arbitrary row's id.
fn recent(conn: &Connection, q: &str, limit: usize) -> Result<Vec<RecentQuery>> {
    let rows = retrieval_log::prefix_matches(conn, SEARCH_CODE, &like_prefix(q))?;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for row in rows {
        if out.len() >= limit {
            break;
        }
        if seen.insert(row.query.to_lowercase()) {
            out.push(RecentQuery {
                query: row.query,
                query_id: row.query_id,
                at: row.at,
            });
        }
    }
    Ok(out)
}

/// `LIKE` prefix pattern for a user-supplied string, built on the shared
/// [`like_escape`] so the wildcard set lives in one place (the query pairs
/// it with `ESCAPE '\'`). Anchored at the start — a completion palette
/// must not offer a query the typed text sits in the middle of.
fn like_prefix(q: &str) -> String {
    format!("{}%", like_escape(q))
}

#[cfg(test)]
#[path = "tests/suggest.rs"]
mod tests;
