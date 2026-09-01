#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `retrieval::unified` against a real store: real memories, a real code
//! index, real SQLite. The two invariants that make `find` trustworthy are
//! order-preservation per domain (so a single-domain run matches that
//! domain's dedicated command) and the document leg's weight actually being
//! read.

use comemory::config::{Config, Paths};
use comemory::retrieval::pipeline::PageWindow;
use comemory::retrieval::scope::{Domain, Domains, Filters, TimeScope};
use comemory::retrieval::unified::{self, fuse_domains};
use comemory::store::connection;
use tempfile::TempDir;

/// A migrated store with three memories whose bodies share a query term but
/// differ in how strongly they match it. Rows AND their FTS index, since
/// the memory leg is lexical here (no vector is supplied).
fn store(dir: &TempDir) -> (Paths, rusqlite::Connection) {
    let paths = Paths::new(dir.path().to_path_buf());
    paths.ensure_dirs().unwrap();
    let conn = connection::open(paths.db_path()).unwrap();
    for (id, body) in [
        ("aaaa1111", "frontmatter is the contract the ranker reads"),
        (
            "bbbb2222",
            "frontmatter round-trip broke on empty tag lists",
        ),
        ("cccc3333", "an unrelated note about swap space"),
    ] {
        seed_memory(&conn, id, body);
    }
    (paths, conn)
}

/// Insert a live `memories` row and index it for FTS.
fn seed_memory(conn: &rusqlite::Connection, id: &str, body: &str) {
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES(?1,'x','note','h',?2,'2026-08-01T00:00:00Z','2026-08-01T00:00:00Z','x.md')",
        rusqlite::params![id, body],
    )
    .expect("seed memory");
    comemory::store::fts::index_memory(conn, id, body, "").expect("index memory");
}

fn filters(scope: &TimeScope, domains: Domains) -> Filters<'_> {
    Filters {
        repo: None,
        kind: None,
        scope,
        domains,
    }
}

#[test]
fn a_memory_only_run_preserves_the_memory_legs_own_order() {
    let dir = TempDir::new().unwrap();
    let (_paths, conn) = store(&dir);
    let cfg = Config::defaults();
    let scope = TimeScope::default();

    let unified_run = unified::find(
        &cfg,
        &conn,
        "frontmatter",
        None,
        filters(&scope, Domains::of(&[Domain::Memory])),
        unified::DomainFilters::default(),
        PageWindow::top_k(&cfg),
    )
    .unwrap();

    // The same route -> rerank -> diversify chain, run directly.
    let candidates = comemory::retrieval::router::route(
        &cfg,
        &conn,
        "frontmatter",
        None,
        filters(&scope, Domains::of(&[Domain::Memory])),
        comemory::retrieval::pipeline::pool_size(
            0,
            cfg.retrieval.top_k,
            cfg.retrieval.max_page_window,
        ),
    )
    .unwrap();
    let reranked =
        comemory::retrieval::rerank::rerank(&conn, &cfg, &candidates, scope.as_of_cutoff())
            .unwrap();
    let direct = comemory::retrieval::diversify::diversify(
        reranked,
        cfg.rank.near_dup_hamming,
        cfg.rank.mmr_lambda,
        comemory::retrieval::pipeline::pool_size(
            0,
            cfg.retrieval.top_k,
            cfg.retrieval.max_page_window,
        ),
    );

    let fused_ids: Vec<&str> = unified_run.hits.iter().map(|h| h.id.as_str()).collect();
    let direct_ids: Vec<&str> = direct
        .iter()
        .take(fused_ids.len())
        .map(|h| h.memory_id.as_str())
        .collect();
    assert_eq!(
        fused_ids, direct_ids,
        "a memory-only find must reproduce the memory leg's own ordering — \
         RRF ranks by position, so a single leg passes through unchanged"
    );
    assert!(
        unified_run
            .hits
            .iter()
            .all(|h| h.domain == fuse_domains::DOMAIN_MEMORY),
        "no other domain may leak into a memory-only run"
    );
}

#[test]
fn rank_in_domain_is_one_based_and_dense_within_a_domain() {
    let dir = TempDir::new().unwrap();
    let (_paths, conn) = store(&dir);
    let cfg = Config::defaults();
    let scope = TimeScope::default();

    let run = unified::find(
        &cfg,
        &conn,
        "frontmatter",
        None,
        filters(&scope, Domains::all()),
        unified::DomainFilters::default(),
        PageWindow::top_k(&cfg),
    )
    .unwrap();

    let mut memory_ranks: Vec<usize> = run
        .hits
        .iter()
        .filter(|h| h.domain == fuse_domains::DOMAIN_MEMORY)
        .map(|h| h.rank_in_domain)
        .collect();
    memory_ranks.sort_unstable();
    let expected: Vec<usize> = (1..=memory_ranks.len()).collect();
    assert_eq!(
        memory_ranks, expected,
        "rank_in_domain numbers a domain's hits 1..n in that domain's own order"
    );
}

#[test]
fn an_empty_corpus_returns_an_empty_ranking_rather_than_an_error() {
    let dir = TempDir::new().unwrap();
    let paths = Paths::new(dir.path().to_path_buf());
    paths.ensure_dirs().unwrap();
    let conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();
    let scope = TimeScope::default();

    let run = unified::find(
        &cfg,
        &conn,
        "nothing here",
        None,
        filters(&scope, Domains::all()),
        unified::DomainFilters::default(),
        PageWindow::top_k(&cfg),
    )
    .unwrap();

    assert!(run.hits.is_empty());
    assert_eq!(run.total, 0);
    assert!(!run.has_more);
}

#[test]
fn excluding_a_domain_skips_its_leg_entirely() {
    let dir = TempDir::new().unwrap();
    let (_paths, conn) = store(&dir);
    let cfg = Config::defaults();
    let scope = TimeScope::default();

    let code_only = unified::find(
        &cfg,
        &conn,
        "frontmatter",
        None,
        filters(&scope, Domains::of(&[Domain::Code])),
        unified::DomainFilters::default(),
        PageWindow::top_k(&cfg),
    )
    .unwrap();

    assert!(
        code_only.hits.is_empty(),
        "the corpus has memories but no indexed code, so a code-only run is empty \
         — the memory leg must not be run and discarded"
    );
}
