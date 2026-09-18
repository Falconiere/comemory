#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The projection reads and the pushed cursor behind `domains::sync::code`, over a
//! real `index-code` run.

use comemory::config::{Config, Paths};
use comemory::store::code_sync::{self, CodeSyncCursor};
use comemory::store::connection;

use crate::test_common::code_sync_fixture as fixture;

#[test]
fn parent_symbols_come_in_source_order_without_chunks() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("data"));
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let tree = fixture::write_ts_repo(home.path());
    fixture::index(&paths, &cfg, &mut conn, &tree);

    let rows = code_sync::parent_symbols_for_file(&conn, fixture::REPO, "src/c.ts").unwrap();
    let names: Vec<&str> = rows.iter().map(|r| r.symbol.as_str()).collect();
    assert_eq!(names, ["gamma"], "{rows:?}");
    assert_eq!(rows[0].lang, "typescript");
    assert!(rows[0].line_start >= 1 && rows[0].line_end >= rows[0].line_start);
}

#[test]
fn import_targets_are_repo_relative_and_sorted() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("data"));
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let tree = fixture::write_ts_repo(home.path());
    fixture::index(&paths, &cfg, &mut conn, &tree);

    let targets = code_sync::import_targets(&conn, fixture::REPO, "src/c.ts").unwrap();
    assert_eq!(targets, ["src/a.ts", "src/b.ts"]);
    assert!(
        code_sync::import_targets(&conn, fixture::REPO, "src/a.ts")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn co_changed_pairs_strip_the_node_prefix() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("data"));
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let tree = fixture::write_ts_repo(home.path());
    fixture::index(&paths, &cfg, &mut conn, &tree);

    let pairs = code_sync::co_changed_pairs(&conn, fixture::REPO).unwrap();
    for (from, to, weight) in &pairs {
        assert!(from.starts_with("src/"), "{from}");
        assert!(to.starts_with("src/"), "{to}");
        assert!(*weight >= 1);
        assert!(from < to, "canonical order: {from} < {to}");
    }
}

#[test]
fn cursor_round_trips_and_tolerates_garbage() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("data"));
    paths.ensure_dirs().unwrap();
    let conn = connection::open(paths.db_path()).unwrap();

    assert_eq!(code_sync::cursor(&conn, "never").unwrap(), None);
    let cursor = CodeSyncCursor {
        pushed_head: Some("abc".into()),
        pushed_mined_commit: None,
        pushed_digest: "d".repeat(64),
        pushed_at: "2026-09-15T12:00:00Z".into(),
    };
    code_sync::set_cursor(&conn, "acme/app", &cursor).unwrap();
    assert_eq!(code_sync::cursor(&conn, "acme/app").unwrap(), Some(cursor));

    comemory::store::schema_meta::upsert(&conn, "code_sync:broken", "not json").unwrap();
    assert_eq!(code_sync::cursor(&conn, "broken").unwrap(), None);
}
