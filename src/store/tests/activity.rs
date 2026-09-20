#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for `src/store/activity.rs` against a real migrated
//! `comemory.db`: insert with its returned id, the filtered newest-first
//! window with its total, the ascending cursor read the stream polls, and the
//! retention delete.

use comemory::store::activity::{self, ActivityFilter, NewActivityRow};
use comemory::store::connection;
use tempfile::tempdir;

fn row<'a>(at: &'a str, command: &'a str, source: &'a str) -> NewActivityRow<'a> {
    NewActivityRow {
        at,
        command,
        source,
        actor: Some("test-host/1.0"),
        repo: Some("comemory"),
        duration_ms: 12,
        ok: true,
        error_code: None,
        summary: Some(r#"{"id":"a1b2c3d4"}"#),
    }
}

#[test]
fn insert_returns_monotonic_ids_and_lists_newest_first_with_total() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();

    let first = activity::insert(&conn, &row("2026-09-20T10:00:00Z", "save", "cli")).unwrap();
    let second = activity::insert(&conn, &row("2026-09-20T11:00:00Z", "find", "mcp")).unwrap();
    let third = activity::insert(&conn, &row("2026-09-20T12:00:00Z", "save", "http")).unwrap();
    assert!(first < second && second < third, "ids are monotonic");

    let (all, total) = activity::list(&conn, &ActivityFilter::default(), 0, 0).unwrap();
    assert_eq!(total, 3);
    assert_eq!(
        all.iter().map(|r| r.id).collect::<Vec<_>>(),
        [third, second, first],
        "newest first"
    );
    assert_eq!(all[0].command, "save");
    assert_eq!(all[0].source, "http");
    assert_eq!(all[0].actor.as_deref(), Some("test-host/1.0"));
    assert_eq!(all[0].summary.as_deref(), Some(r#"{"id":"a1b2c3d4"}"#));
    assert!(all[0].ok);
}

#[test]
fn filters_narrow_by_command_source_and_since_and_report_their_own_total() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();

    activity::insert(&conn, &row("2026-09-20T10:00:00Z", "save", "cli")).unwrap();
    activity::insert(&conn, &row("2026-09-20T11:00:00Z", "find", "mcp")).unwrap();
    let newest = activity::insert(&conn, &row("2026-09-20T12:00:00Z", "save", "http")).unwrap();

    let saves = ActivityFilter {
        command: Some("save"),
        ..ActivityFilter::default()
    };
    let (rows, total) = activity::list(&conn, &saves, 0, 0).unwrap();
    assert_eq!((rows.len(), total), (2, 2));

    let recent_saves = ActivityFilter {
        command: Some("save"),
        since: Some("2026-09-20T11:30:00Z"),
        ..ActivityFilter::default()
    };
    let (rows, total) = activity::list(&conn, &recent_saves, 2, 0).unwrap();
    assert_eq!(total, 1, "total counts the filter, not the page");
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [newest]);

    let future = ActivityFilter {
        since: Some("2026-09-21T00:00:00Z"),
        ..ActivityFilter::default()
    };
    let (rows, total) = activity::list(&conn, &future, 0, 0).unwrap();
    assert!(rows.is_empty());
    assert_eq!(total, 0, "a since past every row is empty, not an error");

    let by_source = ActivityFilter {
        source: Some("mcp"),
        ..ActivityFilter::default()
    };
    let (rows, _) = activity::list(&conn, &by_source, 0, 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].command, "find");
}

#[test]
fn limit_and_offset_page_the_window() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    for hour in 0..5 {
        let at = format!("2026-09-20T1{hour}:00:00Z");
        activity::insert(&conn, &row(&at, "save", "cli")).unwrap();
    }

    let (page, total) = activity::list(&conn, &ActivityFilter::default(), 2, 1).unwrap();
    assert_eq!(total, 5);
    assert_eq!(page.len(), 2);
    assert_eq!(
        page.iter().map(|r| r.at.as_str()).collect::<Vec<_>>(),
        ["2026-09-20T13:00:00Z", "2026-09-20T12:00:00Z"],
        "offset skips the newest row"
    );
}

#[test]
fn since_cursor_returns_ascending_rows_above_the_cursor_and_newest_id_tracks_the_head() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    assert_eq!(
        activity::newest_id(&conn).unwrap(),
        0,
        "an empty table has no cursor"
    );

    let first = activity::insert(&conn, &row("2026-09-20T10:00:00Z", "save", "cli")).unwrap();
    assert_eq!(activity::newest_id(&conn).unwrap(), first);

    let second = activity::insert(&conn, &row("2026-09-20T11:00:00Z", "find", "cli")).unwrap();
    let third = activity::insert(&conn, &row("2026-09-20T12:00:00Z", "save", "cli")).unwrap();

    let after_first = ActivityFilter {
        after_id: Some(first),
        ..ActivityFilter::default()
    };
    let rows = activity::since_cursor(&conn, &after_first, 10).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        [second, third],
        "ascending, so a cursor cannot skip a row"
    );

    let after_head = ActivityFilter {
        after_id: Some(third),
        ..ActivityFilter::default()
    };
    assert!(
        activity::since_cursor(&conn, &after_head, 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn delete_before_evicts_older_rows_and_keeps_the_boundary() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    activity::insert(&conn, &row("2026-08-01T10:00:00Z", "save", "cli")).unwrap();
    activity::insert(&conn, &row("2026-09-01T10:00:00Z", "save", "cli")).unwrap();
    let kept = activity::insert(&conn, &row("2026-09-20T10:00:00Z", "find", "cli")).unwrap();

    let removed = activity::delete_before(&conn, "2026-09-01T10:00:00Z").unwrap();
    assert_eq!(removed, 1, "the cutoff itself is kept");

    let removed = activity::delete_before(&conn, "2026-09-20T00:00:00Z").unwrap();
    assert_eq!(removed, 1);

    let (rows, total) = activity::list(&conn, &ActivityFilter::default(), 0, 0).unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows[0].id, kept);
}

#[test]
fn a_failed_run_round_trips_its_slug_and_a_summary_less_row_reads_back_null() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    activity::insert(
        &conn,
        &NewActivityRow {
            at: "2026-09-20T10:00:00Z",
            command: "save",
            source: "cli",
            actor: None,
            repo: None,
            duration_ms: 3,
            ok: false,
            error_code: Some("bad_request"),
            summary: None,
        },
    )
    .unwrap();

    let (rows, _) = activity::list(&conn, &ActivityFilter::default(), 0, 0).unwrap();
    assert!(!rows[0].ok);
    assert_eq!(rows[0].error_code.as_deref(), Some("bad_request"));
    assert_eq!(rows[0].summary, None);
    assert_eq!(rows[0].actor, None);
    assert_eq!(rows[0].repo, None);
}

#[test]
fn the_source_check_refuses_a_surface_outside_the_vocabulary() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    let err = activity::insert(
        &conn,
        &row("2026-09-20T10:00:00Z", "save", "carrier-pigeon"),
    )
    .expect_err("the CHECK constraint refuses an unknown source");
    assert!(
        format!("{err}").to_lowercase().contains("constraint"),
        "the native SQLite constraint error survives: {err}"
    );
}
