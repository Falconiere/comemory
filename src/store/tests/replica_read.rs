#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::replica_read`] against a real migrated
//! database — cursor paging, the head, kind filtering, and the live digest
//! set the manifest buckets.

use comemory::store::replica_journal::{self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin};
use comemory::store::replica_read;
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

/// Journal one upsert and return its assigned position.
fn accept(conn: &mut Connection, id: &str, kind: &str, key: &str, digest: &str) -> i64 {
    let epoch = replica_journal::stream_epoch(conn).expect("epoch");
    let tx = conn.transaction().expect("tx");
    let sequence = replica_journal::append(
        &tx,
        &epoch,
        &NewOperation {
            operation_id: id,
            entity_kind: kind,
            entity_key: key,
            op: ReplicaOp::Upsert,
            payload: Some(PayloadRef {
                digest,
                bytes: r#"{"body":"payload"}"#,
            }),
            schema_version: 1,
            repository: None,
            origin: ReplicaOrigin::Local,
            at: "2026-09-21T10:00:00Z",
        },
    )
    .expect("append");
    tx.commit().expect("commit");
    sequence
}

#[test]
fn an_empty_journal_has_head_zero_and_an_empty_page() {
    let (_dir, conn) = migrated_db();
    assert_eq!(replica_read::head(&conn).expect("head"), 0);
    assert!(
        replica_read::page(&conn, 0, 100, None)
            .expect("page")
            .is_empty()
    );
    assert!(
        replica_read::kind_digests(&conn)
            .expect("digests")
            .is_empty()
    );
}

#[test]
fn a_page_returns_rows_above_the_cursor_in_order_and_stops_at_the_limit() {
    let (_dir, mut conn) = migrated_db();
    let digests: Vec<String> = (0..4).map(|n| format!("{n}").repeat(64)).collect();
    for (n, digest) in digests.iter().enumerate() {
        accept(
            &mut conn,
            &format!("op-{n}"),
            "memory",
            &format!("key{n}"),
            digest,
        );
    }

    let first = replica_read::page(&conn, 0, 2, None).expect("page");
    assert_eq!(
        first.iter().map(|r| r.sequence).collect::<Vec<_>>(),
        vec![1, 2]
    );

    let next = replica_read::page(&conn, 2, 2, None).expect("page");
    assert_eq!(
        next.iter().map(|r| r.sequence).collect::<Vec<_>>(),
        vec![3, 4]
    );

    // A cursor at head is caught up — an empty page, not a wrapped one.
    assert!(
        replica_read::page(&conn, 4, 2, None)
            .expect("page")
            .is_empty()
    );
    // A cursor above head stays empty rather than replaying history.
    assert!(
        replica_read::page(&conn, 99, 2, None)
            .expect("page")
            .is_empty()
    );
    assert_eq!(replica_read::head(&conn).expect("head"), 4);
}

#[test]
fn a_page_can_be_restricted_to_one_entity_kind() {
    let (_dir, mut conn) = migrated_db();
    let memory_digest = "a".repeat(64);
    let code_digest = "b".repeat(64);
    accept(&mut conn, "op-1", "memory", "a1b2c3d4", &memory_digest);
    accept(
        &mut conn,
        "op-2",
        "code",
        "Falconiere/comemory",
        &code_digest,
    );

    let memories = replica_read::page(&conn, 0, 10, Some("memory")).expect("page");
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].entity_kind, "memory");

    let all = replica_read::page(&conn, 0, 10, None).expect("page");
    assert_eq!(all.len(), 2, "no filter returns every kind");
    let kinds: Vec<String> = replica_read::kind_digests(&conn)
        .expect("digests")
        .into_iter()
        .map(|(kind, _)| kind)
        .collect();
    assert_eq!(kinds, vec!["code".to_string(), "memory".to_string()]);
}

#[test]
fn live_digests_exclude_tombstoned_entities() {
    let (_dir, mut conn) = migrated_db();
    let kept = "a".repeat(64);
    let removed = "b".repeat(64);
    accept(&mut conn, "op-1", "memory", "kept0001", &kept);
    accept(&mut conn, "op-2", "memory", "gone0001", &removed);

    let epoch = replica_journal::stream_epoch(&conn).expect("epoch");
    let tx = conn.transaction().expect("tx");
    replica_journal::append(
        &tx,
        &epoch,
        &NewOperation {
            operation_id: "op-3",
            entity_kind: "memory",
            entity_key: "gone0001",
            op: ReplicaOp::Tombstone,
            payload: None,
            schema_version: 1,
            repository: None,
            origin: ReplicaOrigin::Local,
            at: "2026-09-21T10:05:00Z",
        },
    )
    .expect("tombstone");
    tx.commit().expect("commit");

    assert_eq!(
        replica_read::kind_digests(&conn).expect("digests"),
        vec![("memory".to_string(), vec![kept])],
        "a deleted entity holds nothing to compare"
    );
}

#[test]
fn a_feed_row_keeps_the_payload_it_named_after_the_entity_changes_again() {
    let (_dir, mut conn) = migrated_db();
    let first_digest = "a".repeat(64);
    accept(&mut conn, "op-1", "memory", "a1b2c3d4", &first_digest);

    let second_digest = "b".repeat(64);
    let epoch = replica_journal::stream_epoch(&conn).expect("epoch");
    let tx = conn.transaction().expect("tx");
    replica_journal::append(
        &tx,
        &epoch,
        &NewOperation {
            operation_id: "op-2",
            entity_kind: "memory",
            entity_key: "a1b2c3d4",
            op: ReplicaOp::Upsert,
            payload: Some(PayloadRef {
                digest: &second_digest,
                bytes: r#"{"body":"payload","tags":["added"]}"#,
            }),
            schema_version: 1,
            repository: None,
            origin: ReplicaOrigin::Local,
            at: "2026-09-21T10:10:00Z",
        },
    )
    .expect("second");
    tx.commit().expect("commit");

    let page = replica_read::page(&conn, 0, 10, None).expect("page");
    assert_eq!(page[0].payload.as_deref(), Some(r#"{"body":"payload"}"#));
    assert_eq!(
        page[1].payload.as_deref(),
        Some(r#"{"body":"payload","tags":["added"]}"#),
        "history describes the bytes accepted at that position, not today's"
    );
}

#[test]
fn kind_page_scans_raw_positions_and_continues_past_an_empty_window() {
    let (_dir, mut conn) = migrated_db();
    // Five memories then one document, in feed order.
    for n in 0..5 {
        accept(
            &mut conn,
            &format!("op-m{n}"),
            "memory",
            &format!("m{n}"),
            &format!("{n}").repeat(64),
        );
    }
    accept(
        &mut conn,
        "op-d0",
        "document_revision",
        "d0",
        &"d".repeat(64),
    );

    let (rows, next) = replica_read::scan(&conn, 0, 2, Some("document_revision")).expect("scan");
    assert!(rows.is_empty(), "no document in the first two positions");
    assert_eq!(next, Some(2), "but the reader still moves past them");
    let (rows, next) = replica_read::scan(&conn, 4, 2, Some("document_revision")).expect("scan");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].entity_key, "d0");
    assert_eq!(next, Some(6));
    let (rows, next) = replica_read::scan(&conn, 6, 2, Some("document_revision")).expect("scan");
    assert!(rows.is_empty());
    assert_eq!(next, None, "at the head there is nothing left to scan");
}
