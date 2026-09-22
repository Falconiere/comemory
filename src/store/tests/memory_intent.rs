#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::memory_intent`] against a real migrated
//! database — the marker that says a memory write is in flight, and the two
//! properties the crash recovery depends on: a finished write forgets it, and
//! a rolled-back one does not.

use comemory::store::memory_intent::{self, Intent, IntentKind};
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

fn intent(key: &str, kind: IntentKind, started_at: &str) -> Intent {
    Intent {
        entity_key: key.to_string(),
        kind,
        md_path: format!("memories/{key}-a-real-decision.md"),
        operation_id: format!("op-20260922-{key}"),
        started_at: started_at.to_string(),
    }
}

#[test]
fn a_recorded_intent_is_outstanding_until_it_is_cleared() {
    let (_dir, conn) = migrated_db();
    let recorded = intent("a1b2c3d4", IntentKind::Write, "2026-09-22T10:00:00Z");

    memory_intent::record(&conn, &recorded).expect("record");
    assert_eq!(
        memory_intent::outstanding(&conn).expect("outstanding"),
        vec![recorded.clone()]
    );

    memory_intent::clear(&conn, "a1b2c3d4").expect("clear");
    assert!(
        memory_intent::outstanding(&conn)
            .expect("outstanding")
            .is_empty()
    );
}

#[test]
fn a_second_intent_for_one_memory_supersedes_the_first() {
    let (_dir, conn) = migrated_db();
    memory_intent::record(
        &conn,
        &intent("a1b2c3d4", IntentKind::Write, "2026-09-22T10:00:00Z"),
    )
    .expect("record write");
    let newer = intent("a1b2c3d4", IntentKind::Delete, "2026-09-22T10:05:00Z");
    memory_intent::record(&conn, &newer).expect("record delete");

    let rows = memory_intent::outstanding(&conn).expect("outstanding");
    assert_eq!(
        rows,
        vec![newer],
        "the table holds work in flight, never a history of it"
    );
}

#[test]
fn outstanding_intents_come_back_oldest_first() {
    let (_dir, conn) = migrated_db();
    memory_intent::record(
        &conn,
        &intent("cccccccc", IntentKind::Write, "2026-09-22T12:00:00Z"),
    )
    .expect("record third");
    memory_intent::record(
        &conn,
        &intent("aaaaaaaa", IntentKind::Write, "2026-09-22T10:00:00Z"),
    )
    .expect("record first");
    memory_intent::record(
        &conn,
        &intent("bbbbbbbb", IntentKind::Delete, "2026-09-22T11:00:00Z"),
    )
    .expect("record second");

    let keys: Vec<String> = memory_intent::outstanding(&conn)
        .expect("outstanding")
        .into_iter()
        .map(|row| row.entity_key)
        .collect();
    assert_eq!(keys, ["aaaaaaaa", "bbbbbbbb", "cccccccc"]);
}

#[test]
fn clearing_an_intent_that_is_not_there_is_a_no_op() {
    let (_dir, conn) = migrated_db();
    memory_intent::clear(&conn, "deadbeef").expect("clear is tolerant");
    assert!(
        memory_intent::outstanding(&conn)
            .expect("outstanding")
            .is_empty()
    );
}

#[test]
fn a_rolled_back_transaction_leaves_the_intent_standing() {
    let (_dir, mut conn) = migrated_db();
    let recorded = intent("a1b2c3d4", IntentKind::Write, "2026-09-22T10:00:00Z");
    memory_intent::record(&conn, &recorded).expect("record");

    // The mirror write's own transaction: clearing inside it and then failing
    // must leave the write recorded as unfinished, which is the whole reason
    // `clear` takes the caller's connection.
    let tx = conn.transaction().expect("begin");
    memory_intent::clear(&tx, "a1b2c3d4").expect("clear inside the transaction");
    drop(tx);

    assert_eq!(
        memory_intent::outstanding(&conn).expect("outstanding"),
        vec![recorded],
        "an aborted mirror write must still owe reconciliation"
    );
}

#[test]
fn the_database_refuses_a_kind_outside_the_declared_vocabulary() {
    let (_dir, conn) = migrated_db();
    conn.execute_batch(
        "INSERT INTO memory_write_intent(entity_key, kind, md_path, operation_id, started_at) \
         VALUES ('a1b2c3d4', 'write', 'memories/x.md', 'op-1', '2026-09-22T10:00:00Z'); \
         UPDATE memory_write_intent SET kind = 'archive' WHERE entity_key = 'a1b2c3d4';",
    )
    .expect_err("the CHECK constraint refuses an unknown kind at the database");
}
