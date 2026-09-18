#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::stats::sqlite::StatsDb`].
//!
//! v0.2 unification: `StatsDb::open` now opens `comemory.db` (via
//! `crate::store::connection::open`) instead of the old standalone
//! `stats.db`. The stats tables (`retrieval_log`, `repo_marker`,
//! `index_failures`) are applied by migration `0003_stats_tables`;
//! `feedback` is already present from `0002_v2_tables`.
//!
//! The handle owns no table of its own — #173 moved the `index_failures`
//! bookkeeping it used to delegate into `store::index_failures`, where its
//! own tests live. What remains to assert here is the property every
//! learning writer depends on: `conn()` hands out the one migrated
//! connection the composable writers reuse, so a reward minted through it
//! lands in the same database, without a second `StatsDb` being opened.

use comemory::config::paths::Paths;
use comemory::stats::feedback::record_implicit_used;
use comemory::stats::sqlite::StatsDb;
use comemory::utilities::telemetry::{COACTIVATION_QUERY_ID, PROV_AUTO_COACTIVATION};

use crate::test_common as common;

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
fn conn_is_the_one_connection_the_composable_writers_reuse() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    let db = StatsDb::open(paths.stats_db()).expect("open");

    // `record_implicit_used` takes a bare `&Connection` so callers can mint
    // the reward inside a transaction they already own; borrowing it from
    // the handle must reach the same `comemory.db` the handle opened.
    record_implicit_used(
        db.conn(),
        "aaaaaaa1",
        "2026-09-18T00:00:00.000000000Z",
        PROV_AUTO_COACTIVATION,
        COACTIVATION_QUERY_ID,
    )
    .expect("mint one implicit used");

    let (events, used): (i64, i64) = db
        .conn()
        .query_row(
            "SELECT (SELECT count(*) FROM feedback_events \
              WHERE memory_id='aaaaaaa1' AND provenance=?1), \
             (SELECT used_count FROM feedback WHERE memory_id='aaaaaaa1')",
            [PROV_AUTO_COACTIVATION],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("read back through the same connection");
    assert_eq!(events, 1, "one auto-coactivation event row");
    assert_eq!(used, 1, "counter upserted alongside it");
}
