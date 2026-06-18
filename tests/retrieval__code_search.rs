//! Test mirror for `src/retrieval/code_search.rs`.
//!
//! Exercises the shared [`comemory::retrieval::code_search::search_code_hits`]
//! route → rerank orchestration against a real sqlite-vec-backed connection
//! seeded through the production `code_symbols` + `code_fts` writers (NO
//! mocks): a hitting lexical query returns ranked [`CodeReranked`] results,
//! and a no-match query returns an empty Vec rather than an error.

use comemory::config::Config;
use comemory::retrieval::code_search::search_code_hits;
use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::{connection, fts};
use tempfile::tempdir;

/// Candidate-pool size used by every case (the router default).
const POOL: usize = comemory::retrieval::router::CANDIDATE_POOL;

/// Seed one real `code_symbols` row via the production writer and index
/// its `code_fts` sibling so the lexical leg of `route_code` can reach it.
/// Returns the assigned rowid.
fn seed(conn: &rusqlite::Connection, repo: &str, path: &str, symbol: &str, snippet: &str) -> i64 {
    let id = code_row::insert(
        conn,
        &CodeSymbolRow {
            repo,
            path,
            blob_oid: "oid",
            symbol,
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 10,
            snippet,
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("insert code symbol");
    fts::index_code(conn, id, symbol, snippet, path).expect("index code fts");
    id
}

#[test]
fn hitting_query_returns_ranked_results() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let login = seed(
        &conn,
        "webapp",
        "src/auth/login.rs",
        "login::handle",
        "fn handle() { /* login flow */ }",
    );
    // A second, unrelated symbol that the query should not surface.
    seed(
        &conn,
        "webapp",
        "src/db/pool.rs",
        "pool::checkout",
        "fn checkout() { /* connection pool */ }",
    );

    let cfg = Config::defaults();
    let hits =
        search_code_hits(&cfg, &conn, "login", None, None, None, POOL).expect("search_code_hits");

    assert!(!hits.is_empty(), "a matching query must return ranked hits");
    assert_eq!(
        hits[0].symbol_id,
        login,
        "the login symbol must rank first, got {:?}",
        hits.iter().map(|h| h.symbol_id).collect::<Vec<_>>()
    );
    // Reranked rows carry their full identity row + score parts.
    assert_eq!(hits[0].symbol, "login::handle");
    assert!(
        hits[0].parts.final_score > 0.0,
        "ranked hit must carry a positive final score"
    );
}

#[test]
fn no_match_query_returns_empty_vec_not_error() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    seed(
        &conn,
        "webapp",
        "src/auth/login.rs",
        "login::handle",
        "fn handle() { /* login flow */ }",
    );

    let cfg = Config::defaults();
    // A term that appears in no seeded symbol, and no vector → zero
    // candidates → the zero-candidate WorkingSet guard short-circuits and
    // rerank returns an empty Vec (NOT an error).
    let hits = search_code_hits(
        &cfg,
        &conn,
        "nonexistent_xyzzy_token",
        None,
        None,
        None,
        POOL,
    )
    .expect("search_code_hits must not error on a no-match query");
    assert!(
        hits.is_empty(),
        "a query that hits nothing must return an empty Vec, got {:?}",
        hits.iter().map(|h| h.symbol_id).collect::<Vec<_>>()
    );
}
