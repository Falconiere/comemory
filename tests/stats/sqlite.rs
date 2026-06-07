use comemory::stats::sqlite::StatsDb;
use time::OffsetDateTime;

use super::common;

#[test]
fn open_creates_schema() {
    let sb = common::runner::Sandbox::new();
    let db = StatsDb::open(sb.data_dir().join("stats.db")).unwrap();
    let mut stmt = db
        .conn()
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(tables.iter().any(|t| t == "feedback"));
    assert!(tables.iter().any(|t| t == "retrieval_log"));
    assert!(tables.iter().any(|t| t == "repo_marker"));
    assert!(tables.iter().any(|t| t == "index_failures"));
}

#[test]
fn record_index_failure_increments_count_and_returns_latest() {
    let sb = common::runner::Sandbox::new();
    let db = StatsDb::open(sb.data_dir().join("stats.db")).unwrap();
    assert_eq!(db.index_failure_count().unwrap(), 0);
    assert!(db.last_index_failure().unwrap().is_none());

    let t1 = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let t2 = OffsetDateTime::from_unix_timestamp(1_700_000_300).unwrap();
    db.record_index_failure(t1, "lance: read-only fs").unwrap();
    db.record_index_failure(t2, "embedder: onnx load failed")
        .unwrap();

    assert_eq!(db.index_failure_count().unwrap(), 2);
    let last = db.last_index_failure().unwrap().expect("row exists");
    assert_eq!(last.1, "embedder: onnx load failed");
    assert!(
        last.0.starts_with("2023-"),
        "ts should be ISO 8601 in UTC, got {:?}",
        last.0
    );
}
