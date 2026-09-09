#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/store/rebuild_copy.rs`: the `ATTACH`/copy/`DETACH`
//! entry point and its two attached-DB schema-probe helpers, exercised
//! directly against real databases. The full preservation contract (code
//! index, learning state, run history, document domain all surviving a real
//! `comemory rebuild`) is covered end to end by `tests/api__rebuild__copy.rs`
//! and `tests/cli__rebuild_3.rs`; this file covers the lifecycle helper
//! itself and its `DETACH`-even-on-success behavior.

use comemory::store::connection;
use comemory::store::rebuild_copy::{
    copy_preserved_tables_from_old, old_column_exists, old_table_exists,
};
use rusqlite::Connection;
use tempfile::TempDir;

#[test]
fn old_table_exists_and_old_column_exists_probe_the_attached_schema() {
    let old_dir = TempDir::new().expect("old tempdir");
    let old_path = old_dir.path().join("comemory.db");
    connection::open(&old_path).expect("build a real, fully-migrated old db");

    let new_dir = TempDir::new().expect("new tempdir");
    let conn = connection::open(new_dir.path().join("comemory.db")).expect("open new db");
    conn.execute(
        "ATTACH DATABASE ? AS old",
        rusqlite::params![old_path.to_string_lossy().as_ref()],
    )
    .expect("attach old db");

    assert!(old_table_exists(&conn, "feedback").expect("probe feedback table"));
    assert!(!old_table_exists(&conn, "no_such_table").expect("probe missing table"));
    assert!(
        old_column_exists(&conn, "code_symbols", "rank_score").expect("probe rank_score column")
    );
    assert!(
        !old_column_exists(&conn, "code_symbols", "no_such_column").expect("probe missing column")
    );

    conn.execute_batch("DETACH DATABASE old;")
        .expect("detach old db");
}

#[test]
fn copy_preserved_tables_from_old_copies_a_feedback_row_and_leaves_the_connection_usable() {
    let old_dir = TempDir::new().expect("old tempdir");
    let old_path = old_dir.path().join("comemory.db");
    {
        let old_conn = connection::open(&old_path).expect("build old db");
        old_conn
            .execute(
                "INSERT INTO feedback(memory_id, used_count, irrelevant_count, last_used) \
                 VALUES ('m1', 5, 1, '2026-01-01T00:00:00.000000000Z')",
                [],
            )
            .expect("seed feedback row");
    }

    let new_dir = TempDir::new().expect("new tempdir");
    let mut new_conn = connection::open(new_dir.path().join("comemory.db")).expect("open new db");

    copy_preserved_tables_from_old(&mut new_conn, &old_path).expect("copy succeeds");

    let used: i64 = new_conn
        .query_row(
            "SELECT used_count FROM feedback WHERE memory_id = 'm1'",
            [],
            |r| r.get(0),
        )
        .expect("copied feedback row");
    assert_eq!(used, 5);

    // The connection must be reusable after a successful copy too — a
    // second attach under a fresh alias must succeed, proving `old` was
    // detached.
    new_conn
        .execute(
            "ATTACH DATABASE ? AS old",
            rusqlite::params![old_path.to_string_lossy().as_ref()],
        )
        .expect("old must be detached after a successful copy, so re-attaching succeeds");
    new_conn
        .execute_batch("DETACH DATABASE old;")
        .expect("detach again");
}

#[test]
fn copy_preserved_tables_from_old_is_a_no_op_on_two_freshly_migrated_databases() {
    let old_dir = TempDir::new().expect("old tempdir");
    let old_path = old_dir.path().join("comemory.db");
    connection::open(&old_path).expect("build old db");

    let new_dir = TempDir::new().expect("new tempdir");
    let mut new_conn: Connection =
        connection::open(new_dir.path().join("comemory.db")).expect("open new db");

    copy_preserved_tables_from_old(&mut new_conn, &old_path)
        .expect("copy of two empty, fully-migrated databases must succeed");
}
