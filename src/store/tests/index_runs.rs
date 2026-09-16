#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for `src/store/index_runs.rs` against a real migrated
//! `comemory.db`: insert, the repo-filtered newest-first window with its
//! total, `newest`, and the table's `CHECK` constraints.

use comemory::store::connection;
use comemory::store::index_runs::{self, NewIndexRun};
use tempfile::tempdir;

fn run<'a>(id: &'a str, repo: &'a str, started_at: &'a str, outcome: &'a str) -> NewIndexRun<'a> {
    NewIndexRun {
        id,
        repo,
        root_path: Some("/tmp/repo"),
        mode: "incremental",
        started_at,
        finished_at: started_at,
        duration_ms: 1200,
        files_indexed: 7,
        symbols: 42,
        outcome,
        error: (outcome == "error").then_some("boom"),
    }
}

#[test]
fn insert_list_and_newest_round_trip_with_repo_filter_and_window() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();

    index_runs::insert(
        &conn,
        &run("aaaa000000000001", "a", "2026-09-01T10:00:00Z", "ok"),
    )
    .unwrap();
    index_runs::insert(
        &conn,
        &run("aaaa000000000002", "a", "2026-09-01T11:00:00Z", "error"),
    )
    .unwrap();
    index_runs::insert(
        &conn,
        &run("bbbb000000000001", "b", "2026-09-01T12:00:00Z", "cancelled"),
    )
    .unwrap();
    index_runs::insert(
        &conn,
        &run("aaaa000000000000", "a", "2026-09-01T11:00:00Z", "ok"),
    )
    .unwrap();

    let (all, total) = index_runs::list(&conn, None, 0, 0).unwrap();
    assert_eq!(total, 4);
    assert_eq!(
        all.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        [
            "bbbb000000000001",
            "aaaa000000000000",
            "aaaa000000000002",
            "aaaa000000000001"
        ],
        "newest first"
    );
    assert_eq!(all[2].outcome, "error");
    assert_eq!(all[2].error.as_deref(), Some("boom"));
    assert_eq!(all[0].files_indexed, 7);
    assert_eq!(all[0].symbols, 42);
    assert_eq!(all[0].duration_ms, 1200);

    let (repo_a, total_a) = index_runs::list(&conn, Some("a"), 1, 1).unwrap();
    assert_eq!(
        total_a, 3,
        "the total counts the filtered set, not the window"
    );
    assert_eq!(repo_a.len(), 1);
    assert_eq!(repo_a[0].id, "aaaa000000000002");

    let (repo_a_tail, _) = index_runs::list(&conn, Some("a"), 0, 2).unwrap();
    assert_eq!(repo_a_tail.len(), 1);
    assert_eq!(repo_a_tail[0].id, "aaaa000000000001");
    let (missing, missing_total) = index_runs::list(&conn, Some("missing"), 1, 0).unwrap();
    assert!(missing.is_empty());
    assert_eq!(missing_total, 0);

    let newest = index_runs::newest(&conn).unwrap().expect("a newest row");
    assert_eq!(newest.id, "bbbb000000000001");
    assert_eq!(newest.outcome, "cancelled");
}

#[test]
fn newest_on_an_empty_table_is_none_and_checks_reject_bad_enums() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    assert!(index_runs::newest(&conn).unwrap().is_none());

    let mut bad_mode = run("cccc000000000001", "c", "2026-09-01T10:00:00Z", "ok");
    bad_mode.mode = "partial";
    assert!(index_runs::insert(&conn, &bad_mode).is_err(), "mode CHECK");

    let bad_outcome = run("cccc000000000002", "c", "2026-09-01T10:00:00Z", "meh");
    assert!(
        index_runs::insert(&conn, &bad_outcome).is_err(),
        "outcome CHECK"
    );
}

#[test]
fn filtered_and_global_pages_use_their_order_indexes() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    let filtered_plan: String = conn
        .query_row(
            "EXPLAIN QUERY PLAN SELECT id FROM index_runs WHERE repo = ?1 \
             ORDER BY started_at DESC, id ASC LIMIT ?2 OFFSET ?3",
            rusqlite::params!["a", 10, 0],
            |r| r.get(3),
        )
        .unwrap();
    assert!(
        filtered_plan.contains("idx_index_runs_repo_started"),
        "{filtered_plan}"
    );

    let global_plan: String = conn
        .query_row(
            "EXPLAIN QUERY PLAN SELECT id FROM index_runs \
             ORDER BY started_at DESC, id ASC LIMIT ?1 OFFSET ?2",
            rusqlite::params![10, 0],
            |r| r.get(3),
        )
        .unwrap();
    assert!(
        global_plan.contains("idx_index_runs_started"),
        "{global_plan}"
    );
}
