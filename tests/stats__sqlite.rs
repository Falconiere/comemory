//! Tests for [`comemory::stats::sqlite::StatsDb`].
//!
//! v0.2 unification: `StatsDb::open` now opens `comemory.db` (via
//! `crate::store::connection::open`) instead of the old standalone
//! `stats.db`. The stats tables (`retrieval_log`, `repo_marker`,
//! `index_failures`) are applied by migration `0003_stats_tables`;
//! `feedback` is already present from `0002_v2_tables`.

use comemory::config::paths::Paths;
use comemory::stats::sqlite::StatsDb;
use time::OffsetDateTime;

#[path = "common/mod.rs"]
mod common;

#[test]
fn open_creates_stats_tables_in_comemory_db() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    // stats_db() now returns db_path() — both land in comemory.db.
    let db = StatsDb::open(paths.stats_db()).expect("open");
    assert_eq!(
        paths.stats_db(),
        paths.db_path(),
        "stats_db must alias db_path"
    );

    let mut stmt = db
        .conn()
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .expect("prepare");
    let tables: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query")
        .filter_map(Result::ok)
        .collect();

    // All four stats tables must be present in comemory.db.
    assert!(
        tables.iter().any(|t| t == "feedback"),
        "feedback missing: {tables:?}"
    );
    assert!(
        tables.iter().any(|t| t == "retrieval_log"),
        "retrieval_log missing: {tables:?}"
    );
    assert!(
        tables.iter().any(|t| t == "repo_marker"),
        "repo_marker missing: {tables:?}"
    );
    assert!(
        tables.iter().any(|t| t == "index_failures"),
        "index_failures missing: {tables:?}"
    );
    // comemory.db also has the core memory tables.
    assert!(
        tables.iter().any(|t| t == "memories"),
        "memories missing: {tables:?}"
    );
}

#[test]
fn record_index_failure_increments_count_and_returns_latest() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    let db = StatsDb::open(paths.stats_db()).expect("open");
    assert_eq!(db.index_failure_count().expect("count"), 0);
    assert!(db.last_index_failure().expect("last").is_none());

    let t1 = OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("t1");
    let t2 = OffsetDateTime::from_unix_timestamp(1_700_000_300).expect("t2");
    db.record_index_failure(t1, "lance: read-only fs")
        .expect("record 1");
    db.record_index_failure(t2, "embedder: onnx load failed")
        .expect("record 2");

    assert_eq!(db.index_failure_count().expect("count"), 2);
    let last = db.last_index_failure().expect("last").expect("row exists");
    assert_eq!(last.1, "embedder: onnx load failed");
    assert!(
        last.0.starts_with("2023-"),
        "ts should be ISO 8601 in UTC, got {:?}",
        last.0
    );
}
