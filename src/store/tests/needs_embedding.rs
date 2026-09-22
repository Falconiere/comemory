#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::needs_embedding`] against a real migrated
//! database — the backlog of memories whose imported vector was refused, the
//! reason it was refused, and the count `doctor` reports.

use comemory::store::needs_embedding::{self, Pending, Reason};
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

#[test]
fn a_refused_vector_is_recorded_with_what_arrived() {
    let (_dir, conn) = migrated_db();
    let refused = Pending {
        memory_id: "a1b2c3d4".to_string(),
        reason: Reason::Model,
        model: Some("text-embedding-3-small".to_string()),
        dims: Some(1536),
    };

    needs_embedding::record(&conn, &refused, "2026-09-22T10:00:00Z").expect("record");

    assert_eq!(
        needs_embedding::pending(&conn).expect("pending"),
        vec![refused]
    );
    assert_eq!(needs_embedding::pending(&conn).expect("pending").len(), 1);
}

#[test]
fn an_absent_vector_records_no_model_and_no_dimension() {
    let (_dir, conn) = migrated_db();
    needs_embedding::record(
        &conn,
        &Pending {
            memory_id: "a1b2c3d4".to_string(),
            reason: Reason::Absent,
            model: None,
            dims: None,
        },
        "2026-09-22T10:00:00Z",
    )
    .expect("record");

    let rows = needs_embedding::pending(&conn).expect("pending");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reason, Reason::Absent);
    assert_eq!(rows[0].model, None);
    assert_eq!(rows[0].dims, None);
}

#[test]
fn the_newest_refusal_replaces_the_older_one() {
    let (_dir, conn) = migrated_db();
    needs_embedding::record(
        &conn,
        &Pending {
            memory_id: "a1b2c3d4".to_string(),
            reason: Reason::Absent,
            model: None,
            dims: None,
        },
        "2026-09-22T10:00:00Z",
    )
    .expect("record absent");
    needs_embedding::record(
        &conn,
        &Pending {
            memory_id: "a1b2c3d4".to_string(),
            reason: Reason::Dims,
            model: Some("bge-small-en-v1.5".to_string()),
            dims: Some(512),
        },
        "2026-09-22T10:05:00Z",
    )
    .expect("record dims");

    let rows = needs_embedding::pending(&conn).expect("pending");
    assert_eq!(
        rows.len(),
        1,
        "one row per memory, not a history of refusals"
    );
    assert_eq!(rows[0].reason, Reason::Dims);
    assert_eq!(rows[0].dims, Some(512));
}

#[test]
fn draining_one_memory_leaves_the_rest_of_the_backlog() {
    let (_dir, conn) = migrated_db();
    for (id, at) in [
        ("aaaaaaaa", "2026-09-22T10:00:00Z"),
        ("bbbbbbbb", "2026-09-22T11:00:00Z"),
    ] {
        needs_embedding::record(
            &conn,
            &Pending {
                memory_id: id.to_string(),
                reason: Reason::Absent,
                model: None,
                dims: None,
            },
            at,
        )
        .expect("record");
    }

    needs_embedding::clear(&conn, "aaaaaaaa").expect("clear");

    let ids: Vec<String> = needs_embedding::pending(&conn)
        .expect("pending")
        .into_iter()
        .map(|row| row.memory_id)
        .collect();
    assert_eq!(ids, ["bbbbbbbb"]);
    assert_eq!(needs_embedding::pending(&conn).expect("pending").len(), 1);
}

#[test]
fn an_empty_backlog_counts_zero_rather_than_failing() {
    let (_dir, conn) = migrated_db();
    assert!(needs_embedding::pending(&conn).expect("pending").is_empty());
    assert_eq!(needs_embedding::pending(&conn).expect("pending").len(), 0);
    needs_embedding::clear(&conn, "deadbeef").expect("clear is tolerant");
}

#[test]
fn the_database_refuses_a_reason_outside_the_declared_vocabulary() {
    let (_dir, conn) = migrated_db();
    conn.execute_batch(
        "INSERT INTO memory_needs_embedding(memory_id, reason, recorded_at) \
         VALUES ('a1b2c3d4', 'because', '2026-09-22T10:00:00Z');",
    )
    .expect_err("the CHECK constraint refuses an unknown reason");
}
