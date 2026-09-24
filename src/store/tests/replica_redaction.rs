#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Expiry and erasure of shared events' journal copies against a real
//! migrated database: which payloads each reaches, what survives, and how the
//! two redactions order against each other.

use comemory::store::replica_journal::{
    self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin, stream_epoch,
};
use comemory::store::replica_read::{self, Redaction};
use comemory::store::{connection, replica_redaction};
use rusqlite::Connection;
use tempfile::TempDir;

const CUTOFF: &str = "2026-06-01T00:00:00Z";
const NOW: &str = "2026-09-24T10:00:00Z";

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// Journal one event of `kind` under `event_id` at `at`, and return its digest.
fn journal_event(conn: &Connection, kind: &str, event_id: &str, at: &str) -> String {
    let bytes = format!(r#"{{"at":"{at}","event_id":"{event_id}"}}"#);
    let digest = comemory::utilities::digest::sha256_hex(bytes.as_bytes());
    let epoch = stream_epoch(conn).expect("epoch");
    replica_journal::append(
        conn,
        &epoch,
        &NewOperation {
            operation_id: &format!("op-{event_id}"),
            entity_kind: kind,
            entity_key: event_id,
            op: ReplicaOp::Upsert,
            payload: Some(PayloadRef {
                digest: &digest,
                bytes: &bytes,
            }),
            schema_version: 1,
            repository: Some("Falconiere/comemory"),
            origin: ReplicaOrigin::Local,
            at,
        },
    )
    .expect("append");
    digest
}

fn verdict_row(conn: &Connection, event_id: &str, at: &str) {
    verdict_on(conn, "a1b2c3d4", event_id, at);
}

fn verdict_on(conn: &Connection, memory_id: &str, event_id: &str, at: &str) {
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind, provenance, \
                                     event_id) \
         VALUES ('q-20260924-0badc0de', ?3, 'used', ?2, 'memory', 'manual', ?1)",
        [event_id, at, memory_id],
    )
    .expect("verdict row");
}

#[test]
fn an_event_whose_row_is_about_to_be_evicted_expires_and_a_newer_one_does_not() {
    let (_dir, conn) = migrated_db();
    // The feed position is recent (an import happened today) while the verdict
    // itself is old: the row arm is what reaches it.
    let old = journal_event(&conn, "feedback_event", "ev-old", NOW);
    let fresh = journal_event(&conn, "feedback_event", "ev-fresh", NOW);
    verdict_row(&conn, "ev-old", "2026-01-01T00:00:00Z");
    verdict_row(&conn, "ev-fresh", NOW);

    let expired =
        replica_redaction::redact(&conn, replica_redaction::Reach::PastRetention(CUTOFF), NOW)
            .expect("expire");

    assert_eq!(expired, 1);
    assert_eq!(
        comemory::store::replica_redaction::redaction_of(&conn, &old).expect("old"),
        Some(Redaction::Expired)
    );
    assert_eq!(
        comemory::store::replica_redaction::redaction_of(&conn, &fresh).expect("fresh"),
        None
    );
    let page = replica_read::page(&conn, 0, 10, None).expect("page");
    assert_eq!(page.len(), 2, "every position survives");
    assert!(page[0].payload.is_none() && page[0].payload_digest.as_deref() == Some(&old[..]));
    assert!(
        replica_read::revision(&conn, "feedback_event", "ev-old")
            .expect("revision")
            .is_some(),
        "the dedupe metadata stays"
    );
}

#[test]
fn an_imported_event_never_materialized_expires_by_its_feed_position() {
    let (_dir, conn) = migrated_db();
    let digest = journal_event(
        &conn,
        "activity_event",
        "ev-shown-nowhere",
        "2026-01-01T00:00:00Z",
    );
    let memory = journal_event(&conn, "memory", "a1b2c3d4", "2026-01-01T00:00:00Z");

    replica_redaction::redact(&conn, replica_redaction::Reach::PastRetention(CUTOFF), NOW)
        .expect("expire");

    assert_eq!(
        comemory::store::replica_redaction::redaction_of(&conn, &digest).expect("event"),
        Some(Redaction::Expired)
    );
    assert_eq!(
        comemory::store::replica_redaction::redaction_of(&conn, &memory).expect("memory"),
        None,
        "retention never reaches another kind"
    );
}

#[test]
fn erasure_outranks_expiry_and_expiry_never_relabels_an_erasure() {
    let (_dir, conn) = migrated_db();
    let expired = journal_event(
        &conn,
        "feedback_event",
        "ev-expired",
        "2026-01-01T00:00:00Z",
    );
    let erased = journal_event(&conn, "feedback_event", "ev-erased", "2026-01-01T00:00:00Z");
    verdict_on(&conn, "a1b2c3d4", "ev-erased", NOW);
    verdict_on(&conn, "e5f6a7b8", "ev-expired", NOW);
    let first =
        replica_redaction::redact(&conn, replica_redaction::Reach::VerdictsOn("a1b2c3d4"), NOW)
            .expect("erase");
    assert_eq!(first, 1, "only the purged memory's verdict");
    replica_redaction::redact(&conn, replica_redaction::Reach::PastRetention(CUTOFF), NOW)
        .expect("expire");
    assert_eq!(
        comemory::store::replica_redaction::redaction_of(&conn, &erased).expect("erased"),
        Some(Redaction::Erased),
        "a later expiry leaves the erasure as it was"
    );

    let upgraded =
        replica_redaction::redact(&conn, replica_redaction::Reach::VerdictsOn("e5f6a7b8"), NOW)
            .expect("erase");
    assert_eq!(upgraded, 1);
    assert_eq!(
        comemory::store::replica_redaction::redaction_of(&conn, &expired).expect("expired"),
        Some(Redaction::Erased),
        "a purge is the stronger claim"
    );
}

#[test]
fn a_row_redacted_before_v26_reads_as_erased() {
    let (_dir, conn) = migrated_db();
    let digest = journal_event(&conn, "memory", "a1b2c3d4", NOW);
    conn.execute(
        "UPDATE replica_payload SET bytes = NULL, redacted_at = ?2 WHERE digest = ?1",
        [&digest, NOW],
    )
    .expect("legacy redaction");
    assert_eq!(
        comemory::store::replica_redaction::redaction_of(&conn, &digest).expect("redaction"),
        Some(Redaction::Erased)
    );
}
