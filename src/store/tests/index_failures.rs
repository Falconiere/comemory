#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration coverage for `src/store/index_failures.rs`, moved out of
//! `stats::sqlite::StatsDb`. Driven through the real `StatsDb` methods
//! (`record_index_failure` / `index_failure_count` / `last_index_failure`)
//! rather than the `pub(crate)` store helpers directly, since `StatsDb` is
//! the one path production code and tests use.

use comemory::config::paths::Paths;
use comemory::stats::sqlite::StatsDb;
use tempfile::TempDir;
use time::OffsetDateTime;

fn open_db() -> (StatsDb, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::new(tmp.path());
    let db = StatsDb::open(paths.stats_db()).expect("open stats db");
    (db, tmp)
}

#[test]
fn absent_row_yields_zero_count_and_none_before_any_record() {
    let (db, _tmp) = open_db();
    assert_eq!(db.index_failure_count().expect("count"), 0);
    assert!(db.last_index_failure().expect("last").is_none());
}

#[test]
fn record_then_read_reflects_the_latest_row() {
    let (db, _tmp) = open_db();
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
        "ts is ISO 8601 UTC, got {:?}",
        last.0
    );
}
