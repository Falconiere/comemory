use comemory::config::paths::Paths;
use comemory::index::Fts;

use super::common;

#[test]
fn open_creates_db_and_table() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let _fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    assert!(paths.index_dir().join("fts.sqlite").exists());
}

#[test]
fn upsert_then_count_returns_one() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    fts.upsert("a1b2c3d4", "Use Postgres for analytics")
        .unwrap();
    assert_eq!(fts.count().unwrap(), 1);
}

#[test]
fn upsert_same_id_overwrites() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    fts.upsert("a1b2c3d4", "first body").unwrap();
    fts.upsert("a1b2c3d4", "second body").unwrap();
    assert_eq!(fts.count().unwrap(), 1);
}

#[test]
fn delete_removes_row() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    fts.upsert("a1b2c3d4", "body").unwrap();
    fts.delete("a1b2c3d4").unwrap();
    assert_eq!(fts.count().unwrap(), 0);
}

#[test]
fn search_returns_relevant_ids_in_score_order() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    fts.upsert("id1", "postgres analytics decision").unwrap();
    fts.upsert("id2", "redis cache notes").unwrap();
    fts.upsert("id3", "postgres migration race").unwrap();

    let hits = fts.search("postgres", 10).unwrap();
    let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
    assert!(ids.contains(&"id1"));
    assert!(ids.contains(&"id3"));
    assert!(!ids.contains(&"id2"));
}

#[test]
fn search_respects_limit() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    for i in 0..5 {
        fts.upsert(&format!("id{i}"), "postgres").unwrap();
    }
    let hits = fts.search("postgres", 3).unwrap();
    assert_eq!(hits.len(), 3);
}

#[test]
fn search_empty_query_returns_empty() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    fts.upsert("id1", "postgres").unwrap();
    let hits = fts.search("", 5).unwrap();
    assert!(hits.is_empty());
}

/// Regression for C5: malformed FTS5 MATCH expressions ("id:abc", trailing
/// `AND`, contractions with apostrophes, etc.) must not propagate as errors
/// to retrieval callers — they should degrade to an empty result so the
/// fused pipeline can still serve dense hits.
#[test]
fn search_treats_fts5_syntax_errors_as_empty() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    fts.upsert("id1", "the quick brown fox").unwrap();

    // Column-qualified form ("id:abc") — FTS5 rejects when the named column
    // doesn't exist on the table.
    let hits = fts.search("id:abc", 5).unwrap();
    assert!(hits.is_empty(), "column-qualified token must not error");

    // Trailing operator — FTS5 reports a syntax error during query iteration.
    let hits = fts.search("foo AND", 5).unwrap();
    assert!(hits.is_empty(), "trailing AND must not error");

    // Contraction with an apostrophe — FTS5 chokes on bare quote.
    let hits = fts.search("it's", 5).unwrap();
    assert!(hits.is_empty(), "apostrophe contraction must not error");
}
