use comemory::config::paths::Paths;
use comemory::index::Fts;

use super::common;

#[test]
fn open_creates_db_and_table() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let _fts = Fts::open(paths.fts_db()).unwrap();
    assert!(paths.fts_db().exists());
}

#[test]
fn upsert_then_count_returns_one() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.fts_db()).unwrap();
    fts.upsert("a1b2c3d4", "Use Postgres for analytics")
        .unwrap();
    assert_eq!(fts.count().unwrap(), 1);
}

#[test]
fn upsert_same_id_overwrites() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.fts_db()).unwrap();
    fts.upsert("a1b2c3d4", "first body").unwrap();
    fts.upsert("a1b2c3d4", "second body").unwrap();
    assert_eq!(fts.count().unwrap(), 1);
}

#[test]
fn delete_removes_row() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.fts_db()).unwrap();
    fts.upsert("a1b2c3d4", "body").unwrap();
    fts.delete("a1b2c3d4").unwrap();
    assert_eq!(fts.count().unwrap(), 0);
}

#[test]
fn search_returns_relevant_ids_in_score_order() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.fts_db()).unwrap();
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
    let fts = Fts::open(paths.fts_db()).unwrap();
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
    let fts = Fts::open(paths.fts_db()).unwrap();
    fts.upsert("id1", "postgres").unwrap();
    let hits = fts.search("", 5).unwrap();
    assert!(hits.is_empty());
}

/// Regression for C5: malformed FTS5 MATCH expressions (trailing `AND`,
/// contractions with apostrophes, etc.) must not propagate as errors to
/// retrieval callers — they should degrade to an empty result so the fused
/// pipeline can still serve dense hits.
///
/// Column-qualified forms like `"id:abc"` are **not** in this class: see
/// [`search_propagates_column_filter_as_err`] for the contract on schema
/// mismatches.
#[test]
fn search_treats_fts5_syntax_errors_as_empty() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.fts_db()).unwrap();
    fts.upsert("id1", "the quick brown fox").unwrap();

    // Trailing operator — FTS5 reports a syntax error during query iteration.
    let hits = fts.search("foo AND", 5).unwrap();
    assert!(hits.is_empty(), "trailing AND must not error");

    // Contraction with an apostrophe — FTS5 chokes on bare quote.
    let hits = fts.search("it's", 5).unwrap();
    assert!(hits.is_empty(), "apostrophe contraction must not error");
}

/// Regression for G1: a column-qualified MATCH expression against a genuinely
/// missing column (e.g. `"nonexistent:foo"`) is a schema/usage error, not a
/// parse error. The FTS5 search path must propagate it as `Err` rather than
/// swallowing it as an empty result, so callers can spot the mismatch
/// instead of silently degrading every column-filtered query to zero hits.
///
/// (`"id:abc"` against the live schema tokenizes as a literal `id:abc` token
/// and matches no rows — it does *not* exercise the schema-error path.)
#[test]
fn search_propagates_column_filter_as_err() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.fts_db()).unwrap();
    fts.upsert("id1", "the quick brown fox").unwrap();

    let result = fts.search("nonexistent_column:foo", 5);
    assert!(
        result.is_err(),
        "column-qualified MATCH against missing column must propagate as Err, got {result:?}"
    );
}
