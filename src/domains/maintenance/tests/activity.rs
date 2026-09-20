#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirrors `src/domains/maintenance/activity.rs` against a real store: the
//! filters, the page/total split, the rollups, the cursor, and the empty
//! answer a data dir with no database gets without acquiring one.

use comemory::config::{Config, Paths};
use comemory::domains::maintenance::activity;
use comemory::store::activity::{NewActivityRow, insert};
use comemory::store::connection;
use comemory::utilities::context::Ctx;

fn row<'a>(at: &'a str, command: &'a str, source: &'a str) -> NewActivityRow<'a> {
    NewActivityRow {
        at,
        command,
        source,
        actor: Some("acme-console/3"),
        repo: Some("demo"),
        duration_ms: 12,
        ok: true,
        error_code: None,
        summary: Some(r#"{"id":"a1b2c3d4"}"#),
    }
}

fn request() -> activity::Request {
    activity::Request::default()
}

#[test]
fn an_empty_data_dir_answers_empty_and_stays_empty() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let out = activity::run(&mut ctx, request()).unwrap();

    assert!(out.items.is_empty());
    assert_eq!(out.total, 0);
    assert!(out.rollups.is_empty());
    assert_eq!(out.cursor, 0);
    assert!(
        !paths.db_path().exists(),
        "reading the feed must not create comemory.db"
    );
}

#[test]
fn a_page_reports_its_own_total_and_the_tables_newest_cursor() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    for hour in 0..4 {
        let at = format!("2026-09-20T1{hour}:00:00Z");
        insert(&conn, &row(&at, "save", "cli")).unwrap();
    }
    let newest = insert(&conn, &row("2026-09-20T18:00:00Z", "find", "mcp")).unwrap();
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let out = activity::run(
        &mut ctx,
        activity::Request {
            limit: Some(2),
            ..request()
        },
    )
    .unwrap();

    assert_eq!(out.items.len(), 2, "the page is the requested size");
    assert_eq!(out.total, 5, "the total counts every matching row");
    assert_eq!(out.items[0].command, "find", "newest first");
    assert_eq!(out.cursor, newest, "the cursor is the table's newest id");
    assert_eq!(
        out.items[0].summary.as_ref().unwrap()["id"],
        "a1b2c3d4",
        "the stored summary comes back as JSON, not as text"
    );
}

#[test]
fn filters_narrow_the_page_the_total_and_the_rollups_together() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    insert(&conn, &row("2026-09-20T10:00:00Z", "save", "cli")).unwrap();
    insert(&conn, &row("2026-09-20T12:00:00Z", "save", "http")).unwrap();
    insert(&conn, &row("2026-09-20T13:00:00Z", "find", "cli")).unwrap();
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let out = activity::run(
        &mut ctx,
        activity::Request {
            command: Some("save".into()),
            since: Some("2026-09-20T11:00:00Z".into()),
            ..request()
        },
    )
    .unwrap();

    assert_eq!(out.items.len(), 1);
    assert_eq!(out.total, 1);
    assert_eq!(out.items[0].source, "http");
    assert_eq!(out.rollups.len(), 1, "rollups follow the same filter");
    assert_eq!(out.rollups[0].command, "save");
    assert_eq!((out.rollups[0].runs, out.rollups[0].errors), (1, 0));
}

#[test]
fn a_since_past_every_row_is_an_empty_page_with_a_live_cursor() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let newest = insert(&conn, &row("2026-09-20T10:00:00Z", "save", "cli")).unwrap();
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let out = activity::run(
        &mut ctx,
        activity::Request {
            since: Some("2026-09-21T00:00:00Z".into()),
            ..request()
        },
    )
    .unwrap();

    assert!(out.items.is_empty());
    assert_eq!(out.total, 0);
    assert!(out.rollups.is_empty());
    assert_eq!(
        out.cursor, newest,
        "an empty page still hands a client where to start streaming"
    );
}

#[test]
fn an_oversized_limit_is_clamped_to_the_maximum_page() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    for i in 0..5 {
        let at = format!("2026-09-20T10:00:0{i}Z");
        insert(&conn, &row(&at, "save", "cli")).unwrap();
    }
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let out = activity::run(
        &mut ctx,
        activity::Request {
            limit: Some(10_000),
            ..request()
        },
    )
    .unwrap();

    assert_eq!(
        out.items.len(),
        5,
        "fewer rows than the clamp, so all of them"
    );

    // And the clamp itself: with more rows than `MAX_LIMIT`, a caller asking
    // for everything gets exactly the ceiling.
    for i in 0..activity::MAX_LIMIT {
        let at = format!("2026-09-20T11:00:00.{i:04}Z");
        insert(&conn, &row(&at, "save", "cli")).unwrap();
    }
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let out = activity::run(
        &mut ctx,
        activity::Request {
            limit: Some(10_000),
            ..request()
        },
    )
    .unwrap();
    assert_eq!(out.items.len(), activity::MAX_LIMIT);
    assert_eq!(
        out.total,
        activity::MAX_LIMIT + 5,
        "the total is not clamped"
    );
}

#[test]
fn an_unparsable_summary_is_dropped_without_failing_the_page() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    insert(
        &conn,
        &NewActivityRow {
            summary: Some("{not json"),
            ..row("2026-09-20T10:00:00Z", "save", "cli")
        },
    )
    .unwrap();
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let out = activity::run(&mut ctx, request()).unwrap();

    assert_eq!(out.items.len(), 1, "the run is still reported");
    assert!(
        out.items[0].summary.is_none(),
        "only its summary is dropped"
    );
}
