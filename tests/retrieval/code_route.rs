//! Test mirror for `src/retrieval/code_route.rs`.
//!
//! Exercises [`comemory::retrieval::code_route::route_code`] against a
//! real sqlite-vec-backed connection: pure-lexical, pure-vector
//! (threshold floor), hybrid RRF fusion, and the repo / lang filters.

use comemory::config::Config;
use comemory::retrieval::code_route::route_code;
use comemory::retrieval::router::Source;
use comemory::store::{connection, fts, vector};
use tempfile::tempdir;

/// Code vector dim baked into the `code_vec` DDL (`schema_meta.code_vector_dim`).
const CODE_DIM: usize = 768;

/// A real 768-dim unit vector with a single `1.0` component. Two distinct
/// basis vectors are exactly orthogonal (cosine similarity 0.0), which is
/// what the threshold test needs.
fn basis(idx: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; CODE_DIM];
    v[idx] = 1.0;
    v
}

/// Insert one `code_symbols` row plus its `code_fts` sibling.
fn seed_symbol(
    conn: &rusqlite::Connection,
    id: i64,
    repo: &str,
    lang: &str,
    symbol: &str,
    snippet: &str,
    path: &str,
) {
    conn.execute(
        "INSERT INTO code_symbols\
            (id,repo,path,blob_oid,symbol,kind,lang,line_start,line_end,snippet,simhash,indexed_at) \
         VALUES(?1,?2,?3,'oid',?4,'function',?5,1,10,?6,0,'t')",
        rusqlite::params![id, repo, path, symbol, lang, snippet],
    )
    .expect("seed code symbol");
    fts::index_code(conn, id, symbol, snippet, path).expect("index code fts");
}

#[test]
fn lexical_query_returns_bm25_hits_without_vector() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_symbol(
        &conn,
        1,
        "webapp",
        "rust",
        "login::handle",
        "fn handle() { /* login flow */ }",
        "src/auth/login.rs",
    );

    let cfg = Config::defaults();
    let hits = route_code(&cfg, &conn, "login", None, None, None).expect("route");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].symbol_id, 1);
    assert_eq!(hits[0].source, Source::Lexical);
}

#[test]
fn hybrid_fuses_both_legs() {
    // Symbol 1 is reachable only lexically (no vector); symbol 2 is
    // reachable only via ANN (its snippet never mentions the query term).
    // The fused result must contain both, tagged Hybrid.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_symbol(
        &conn,
        1,
        "webapp",
        "rust",
        "login::handle",
        "fn handle() { /* login flow */ }",
        "src/auth/lexonly.rs",
    );
    seed_symbol(
        &conn,
        2,
        "webapp",
        "rust",
        "session::refresh",
        "fn refresh() { /* token rotation */ }",
        "src/auth/vecside.rs",
    );
    vector::insert_code(&conn, 2, &basis(0)).expect("insert code vec");

    let cfg = Config::defaults();
    let q = basis(0);
    let hits = route_code(&cfg, &conn, "login", Some(&q), None, None).expect("route");
    let ids: Vec<i64> = hits.iter().map(|h| h.symbol_id).collect();
    assert!(ids.contains(&1), "lexical leg must contribute, got {ids:?}");
    assert!(ids.contains(&2), "ANN leg must contribute, got {ids:?}");
    assert!(
        hits.iter().all(|h| h.source == Source::Hybrid),
        "fused hits must be tagged Hybrid"
    );
}

#[test]
fn ann_hits_below_code_threshold_are_dropped() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_symbol(
        &conn,
        1,
        "webapp",
        "rust",
        "session::refresh",
        "fn refresh() {}",
        "src/auth/session.rs",
    );
    vector::insert_code(&conn, 1, &basis(0)).expect("insert code vec");

    let cfg = Config::defaults();
    // Orthogonal query vector: cosine similarity 0.0 < code_threshold 0.5,
    // so the vector-only route must come back empty.
    let q = basis(1);
    let hits = route_code(&cfg, &conn, "", Some(&q), None, None).expect("route");
    assert!(
        hits.is_empty(),
        "sub-threshold ANN hit must be dropped, got {hits:?}",
        hits = hits.iter().map(|h| h.symbol_id).collect::<Vec<_>>()
    );

    // Positive control: the matching vector clears the floor and is tagged
    // Vector (empty query → no lexical leg).
    let q = basis(0);
    let hits = route_code(&cfg, &conn, "", Some(&q), None, None).expect("route");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].symbol_id, 1);
    assert_eq!(hits[0].source, Source::Vector);
}

#[test]
fn lang_filter_restricts_both_legs() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_symbol(
        &conn,
        1,
        "webapp",
        "rust",
        "config::parse",
        "fn parse() { /* config */ }",
        "src/config.rs",
    );
    seed_symbol(
        &conn,
        2,
        "webapp",
        "python",
        "parse_config",
        "def parse_config(): pass",
        "tools/config.py",
    );
    vector::insert_code(&conn, 1, &basis(0)).expect("vec rust");
    vector::insert_code(&conn, 2, &basis(0)).expect("vec python");

    let cfg = Config::defaults();
    let q = basis(0);
    let hits = route_code(&cfg, &conn, "config", Some(&q), None, Some("rust")).expect("route");
    assert!(!hits.is_empty(), "rust symbol must survive the lang filter");
    assert!(
        hits.iter().all(|h| h.symbol_id == 1),
        "lang=rust must drop the python symbol, got {:?}",
        hits.iter().map(|h| h.symbol_id).collect::<Vec<_>>()
    );
}

#[test]
fn repo_filter_restricts_both_legs() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_symbol(
        &conn,
        1,
        "frontend",
        "rust",
        "config::parse",
        "fn parse() { /* config */ }",
        "src/config.rs",
    );
    seed_symbol(
        &conn,
        2,
        "backend",
        "rust",
        "config::load",
        "fn load() { /* config */ }",
        "src/settings.rs",
    );
    vector::insert_code(&conn, 1, &basis(0)).expect("vec frontend");
    vector::insert_code(&conn, 2, &basis(0)).expect("vec backend");

    let cfg = Config::defaults();
    let q = basis(0);
    let hits = route_code(&cfg, &conn, "config", Some(&q), Some("backend"), None).expect("route");
    assert!(
        !hits.is_empty(),
        "backend symbol must survive the repo filter"
    );
    assert!(
        hits.iter().all(|h| h.symbol_id == 2),
        "repo=backend must drop the frontend symbol, got {:?}",
        hits.iter().map(|h| h.symbol_id).collect::<Vec<_>>()
    );
}

#[test]
fn empty_query_and_no_vector_returns_empty() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_symbol(
        &conn,
        1,
        "webapp",
        "rust",
        "login::handle",
        "fn handle() {}",
        "src/auth/login.rs",
    );

    let cfg = Config::defaults();
    let hits = route_code(&cfg, &conn, "   ", None, None, None).expect("route");
    assert!(hits.is_empty(), "no query + no vector must return nothing");
}
