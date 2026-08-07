#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/source/mirror.rs`.

use std::path::PathBuf;

use comemory::source::mirror::{self, MirrorReport};
use comemory::source::{SourceEntry, SourceId, SourceKind};
use comemory::store::connection;
use comemory::store::sources;
use rusqlite::Connection;
use tempfile::TempDir;
use time::OffsetDateTime;

fn open_db() -> (Connection, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");
    (conn, tmp)
}

/// Build a synthetic entry — mirror reconciliation never touches the
/// filesystem, so an arbitrary (non-existent) canonical path is fine.
fn entry(path: &str, kind: SourceKind, repo: Option<&str>) -> SourceEntry {
    let canonical_path = PathBuf::from(path);
    let now = OffsetDateTime::now_utc();
    SourceEntry {
        id: SourceId::from_canonical_path(&canonical_path),
        canonical_path,
        kind,
        repo: repo.map(str::to_string),
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn reconcile_adds_new_entries() {
    let (conn, _tmp) = open_db();
    let entries = vec![
        entry("/srcs/a", SourceKind::Dir, Some("repo-a")),
        entry("/srcs/b.txt", SourceKind::File, None),
    ];

    let report = mirror::reconcile(&conn, &entries).expect("reconcile");
    assert_eq!(
        report,
        MirrorReport {
            added: 2,
            updated: 0,
            removed: 0,
            total: 2,
        }
    );
    assert_eq!(sources::list(&conn).expect("list").len(), 2);
}

#[test]
fn reconcile_updates_existing_entries_in_place() {
    let (conn, _tmp) = open_db();
    let mut a = entry("/srcs/a", SourceKind::Dir, None);
    mirror::reconcile(&conn, std::slice::from_ref(&a)).expect("first reconcile");

    a.repo = Some("now-labelled".to_string());
    a.updated_at = OffsetDateTime::now_utc();
    let report = mirror::reconcile(&conn, std::slice::from_ref(&a)).expect("second reconcile");

    assert_eq!(report.added, 0);
    assert_eq!(report.updated, 1);
    let row = sources::get(&conn, a.id.as_str())
        .expect("get")
        .expect("row exists");
    assert_eq!(row.repo.as_deref(), Some("now-labelled"));
}

#[test]
fn reconcile_deletes_rows_missing_from_toml() {
    let (conn, _tmp) = open_db();
    let a = entry("/srcs/a", SourceKind::Dir, None);
    let b = entry("/srcs/b", SourceKind::Dir, None);
    mirror::reconcile(&conn, &[a.clone(), b.clone()]).expect("seed both");

    let report = mirror::reconcile(&conn, std::slice::from_ref(&a)).expect("drop b");
    assert_eq!(report.removed, 1);
    assert_eq!(report.total, 1);
    assert!(sources::get(&conn, b.id.as_str()).expect("get").is_none());
    assert!(sources::get(&conn, a.id.as_str()).expect("get").is_some());
}

/// Reachability status is set outside the registry (a future discovery
/// pass); a reconcile must never reset it back to `active` behind that
/// pass's back.
#[test]
fn reconcile_preserves_status_set_outside_reconcile() {
    let (conn, _tmp) = open_db();
    let a = entry("/srcs/a", SourceKind::Dir, None);
    mirror::reconcile(&conn, std::slice::from_ref(&a)).expect("seed");
    conn.execute(
        "UPDATE source_roots SET status = 'unreachable' WHERE id = ?1",
        [a.id.as_str()],
    )
    .expect("mark unreachable");

    mirror::reconcile(&conn, std::slice::from_ref(&a)).expect("reconcile again");

    let row = sources::get(&conn, a.id.as_str())
        .expect("get")
        .expect("row exists");
    assert_eq!(row.status, "unreachable");
}
