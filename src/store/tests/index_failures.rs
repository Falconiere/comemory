#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration coverage for `src/store/index_failures.rs`.
//!
//! Driven through the module's own `record` / `count` / `latest` helpers
//! over a real migrated `comemory.db`. They used to be reached through
//! `stats::sqlite::StatsDb`'s three delegating methods; #173 folded the
//! timestamp formatting and the `usize` clamp in here, so this module is now
//! the one path production code and tests use.

use comemory::config::paths::Paths;
use comemory::store::{Connection, connection, index_failures};
use tempfile::TempDir;
use time::OffsetDateTime;

fn open_db() -> (Connection, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::new(tmp.path());
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.stats_db()).expect("open comemory.db");
    (conn, tmp)
}

#[test]
fn absent_row_yields_zero_count_and_none_before_any_record() {
    let (conn, _tmp) = open_db();
    assert_eq!(index_failures::count(&conn).expect("count"), 0);
    assert!(index_failures::latest(&conn).expect("latest").is_none());
}

#[test]
fn record_then_read_reflects_the_latest_row() {
    let (conn, _tmp) = open_db();
    let t1 = OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("t1");
    let t2 = OffsetDateTime::from_unix_timestamp(1_700_000_300).expect("t2");
    index_failures::record(&conn, t1, "lance: read-only fs").expect("record 1");
    index_failures::record(&conn, t2, "embedder: onnx load failed").expect("record 2");

    assert_eq!(index_failures::count(&conn).expect("count"), 2);
    let last = index_failures::latest(&conn)
        .expect("latest")
        .expect("row exists");
    assert_eq!(last.1, "embedder: onnx load failed");
    assert!(
        last.0.starts_with("2023-"),
        "ts is ISO 8601 UTC, got {:?}",
        last.0
    );
}
