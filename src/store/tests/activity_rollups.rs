#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for `src/store/activity_rollups.rs` against a real migrated
//! `comemory.db`: per-command run and error counts, the percentile pair, and
//! the filter the snapshot route hands it.

use comemory::store::activity::{self, ActivityFilter, NewActivityRow};
use comemory::store::activity_rollups;
use comemory::store::connection;
use tempfile::tempdir;

fn row<'a>(at: &'a str, command: &'a str, duration_ms: i64, ok: bool) -> NewActivityRow<'a> {
    NewActivityRow {
        at,
        command,
        source: "cli",
        actor: None,
        repo: Some("comemory"),
        duration_ms,
        ok,
        error_code: (!ok).then_some("internal"),
        summary: None,
    }
}

#[test]
fn rollups_count_runs_and_errors_per_command_busiest_first() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    for (i, duration) in [10_i64, 20, 30, 40].into_iter().enumerate() {
        let at = format!("2026-09-20T1{i}:00:00Z");
        activity::insert(&conn, &row(&at, "find", duration, i != 3)).unwrap();
    }
    activity::insert(&conn, &row("2026-09-20T09:00:00Z", "save", 5, true)).unwrap();

    let rolled = activity_rollups::rollups(&conn, &ActivityFilter::default()).unwrap();
    assert_eq!(
        rolled
            .iter()
            .map(|r| r.command.as_str())
            .collect::<Vec<_>>(),
        ["find", "save"],
        "busiest command first"
    );
    assert_eq!((rolled[0].runs, rolled[0].errors), (4, 1));
    assert_eq!((rolled[1].runs, rolled[1].errors), (1, 0));
}

#[test]
fn percentiles_are_nearest_rank_over_the_sampled_durations() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    for duration in 1..=100_i64 {
        let at = format!("2026-09-20T10:00:{:02}Z", duration % 60);
        activity::insert(&conn, &row(&at, "search", duration, true)).unwrap();
    }

    let rolled = activity_rollups::rollups(&conn, &ActivityFilter::default()).unwrap();
    assert_eq!(rolled.len(), 1);
    assert_eq!(rolled[0].runs, 100);
    assert_eq!(rolled[0].p50_ms, 50);
    assert_eq!(rolled[0].p95_ms, 95);
}

#[test]
fn a_single_run_reports_itself_and_an_empty_window_rolls_up_to_nothing() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    assert!(
        activity_rollups::rollups(&conn, &ActivityFilter::default())
            .unwrap()
            .is_empty(),
        "an empty table rolls up to no commands, not to a zero row"
    );

    activity::insert(&conn, &row("2026-09-20T10:00:00Z", "save", 42, true)).unwrap();
    let rolled = activity_rollups::rollups(&conn, &ActivityFilter::default()).unwrap();
    assert_eq!((rolled[0].p50_ms, rolled[0].p95_ms), (42, 42));
}

#[test]
fn the_caller_filter_narrows_the_rollup_window() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    activity::insert(&conn, &row("2026-09-20T10:00:00Z", "save", 10, true)).unwrap();
    activity::insert(&conn, &row("2026-09-20T12:00:00Z", "save", 30, false)).unwrap();
    activity::insert(&conn, &row("2026-09-20T12:00:00Z", "find", 30, true)).unwrap();

    let recent = ActivityFilter {
        since: Some("2026-09-20T11:00:00Z"),
        command: Some("save"),
        ..ActivityFilter::default()
    };
    let rolled = activity_rollups::rollups(&conn, &recent).unwrap();
    assert_eq!(rolled.len(), 1, "only the filtered command appears");
    assert_eq!((rolled[0].runs, rolled[0].errors), (1, 1));
    assert_eq!(rolled[0].p50_ms, 30);
}
