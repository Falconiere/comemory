#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`insert`]/[`list`] against a real migrated `comemory.db`.

use comemory::store::{connection, eval_runs};

fn open() -> rusqlite::Connection {
    let dir = tempfile::tempdir().expect("tempdir");
    connection::open(dir.path().join("comemory.db")).expect("open + migrate")
}

#[test]
fn insert_writes_a_readable_row() {
    let conn = open();
    eval_runs::insert(
        &conn,
        &eval_runs::NewRun {
            id: "abcdef0123456789",
            kind: "eval",
            at: "2026-08-31T10:05:00.000000000Z",
            golden_pairs: 10,
            k: 3,
            recall: 0.8,
            mrr: 0.6,
            knobs: "{\"rrf_k\":60.0}",
            applied: false,
        },
    )
    .expect("insert eval_runs row");

    let rows = eval_runs::list(&conn, 10).expect("list rows");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.id, "abcdef0123456789");
    assert_eq!(row.kind, "eval");
    assert_eq!(row.golden_pairs, 10);
    assert_eq!(row.k, 3);
    assert_eq!(row.recall, 0.8);
    assert_eq!(row.mrr, 0.6);
    assert_eq!(row.knobs["rrf_k"].as_f64(), Some(60.0));
    assert!(!row.applied);
    assert!(!row.discarded, "a fresh row defaults to not discarded");
}

/// Seed one row and hand back its id, so the flag tests below stay short.
fn seed(conn: &rusqlite::Connection, id: &str) {
    eval_runs::insert(
        conn,
        &eval_runs::NewRun {
            id,
            kind: "tune",
            at: "2026-09-01T09:00:00.000000000Z",
            golden_pairs: 12,
            k: 3,
            recall: 0.7,
            mrr: 0.55,
            knobs: "{\"rrf_k\":45.0}",
            applied: false,
        },
    )
    .expect("insert eval_runs row");
}

#[test]
fn get_reads_one_row_and_reports_an_unknown_id_as_none() {
    let conn = open();
    seed(&conn, "0123456789abcdef");

    let row = eval_runs::get(&conn, "0123456789abcdef")
        .expect("get row")
        .expect("row exists");
    assert_eq!(row.kind, "tune");
    assert_eq!(row.knobs["rrf_k"].as_f64(), Some(45.0));
    assert!(!row.applied);
    assert!(!row.discarded);

    assert!(
        eval_runs::get(&conn, "nosuchrun00000000")
            .expect("get missing")
            .is_none()
    );
}

#[test]
fn set_applied_and_set_discarded_flip_exactly_their_own_flag() {
    let conn = open();
    seed(&conn, "aaaa000000000000");
    seed(&conn, "bbbb000000000000");

    eval_runs::set_applied(&conn, "aaaa000000000000").expect("set applied");
    eval_runs::set_discarded(&conn, "bbbb000000000000").expect("set discarded");

    let applied = eval_runs::get(&conn, "aaaa000000000000")
        .expect("get")
        .expect("row");
    assert!(applied.applied);
    assert!(!applied.discarded, "apply must not also discard");

    let discarded = eval_runs::get(&conn, "bbbb000000000000")
        .expect("get")
        .expect("row");
    assert!(discarded.discarded);
    assert!(!discarded.applied, "discard must not also apply");
}

#[test]
fn setting_a_flag_on_an_unknown_id_is_not_found() {
    let conn = open();
    let err = eval_runs::set_applied(&conn, "missing0000000000").expect_err("unknown id");
    assert!(
        matches!(err, comemory::errors::Error::NotFound(_)),
        "expected NotFound, got {err:?}"
    );
    let err = eval_runs::set_discarded(&conn, "missing0000000000").expect_err("unknown id");
    assert!(matches!(err, comemory::errors::Error::NotFound(_)));
}

#[test]
fn list_orders_newest_first_and_respects_limit() {
    let conn = open();
    for (id, at) in [
        ("id-1", "2026-08-30T10:00:00.000000000Z"),
        ("id-2", "2026-08-31T10:00:00.000000000Z"),
        ("id-3", "2026-08-29T10:00:00.000000000Z"),
    ] {
        eval_runs::insert(
            &conn,
            &eval_runs::NewRun {
                id,
                kind: "tune",
                at,
                golden_pairs: 5,
                k: 3,
                recall: 0.5,
                mrr: 0.5,
                knobs: "{}",
                applied: true,
            },
        )
        .expect("insert row");
    }

    let rows = eval_runs::list(&conn, 2).expect("list rows");
    assert_eq!(rows.len(), 2, "limit must cap the row count");
    assert_eq!(rows[0].id, "id-2", "newest at wins first");
    assert_eq!(rows[1].id, "id-1");
}

#[test]
fn list_on_an_empty_table_returns_an_empty_vec() {
    let conn = open();
    let rows = eval_runs::list(&conn, 20).expect("list empty table");
    assert!(rows.is_empty());
}
