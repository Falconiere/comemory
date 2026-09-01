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

    let (all, total) = index_runs::list(&conn, None, 0, 0).unwrap();
    assert_eq!(total, 3);
    assert_eq!(
        all.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["bbbb000000000001", "aaaa000000000002", "aaaa000000000001"],
        "newest first"
    );
    assert_eq!(all[1].outcome, "error");
    assert_eq!(all[1].error.as_deref(), Some("boom"));
    assert_eq!(all[0].files_indexed, 7);
    assert_eq!(all[0].symbols, 42);
    assert_eq!(all[0].duration_ms, 1200);

    let (repo_a, total_a) = index_runs::list(&conn, Some("a"), 1, 1).unwrap();
    assert_eq!(
        total_a, 2,
        "the total counts the filtered set, not the window"
    );
    assert_eq!(repo_a.len(), 1);
    assert_eq!(repo_a[0].id, "aaaa000000000001");

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
