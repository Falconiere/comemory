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

    assert_eq!(expired.len(), 1);
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
    assert_eq!(first.len(), 1, "only the purged memory's verdict");
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
    assert_eq!(upgraded.len(), 1);
    assert_eq!(
        comemory::store::replica_redaction::redaction_of(&conn, &expired).expect("expired"),
        Some(Redaction::Erased),
        "a purge is the stronger claim"
    );
}

#[test]
fn a_digest_a_pending_outbox_operation_owes_is_exempt_from_retention() {
    let (_dir, conn) = migrated_db();
    let owed = journal_event(&conn, "feedback_event", "ev-owed", NOW);
    let unowed = journal_event(&conn, "feedback_event", "ev-unowed", NOW);
    verdict_row(&conn, "ev-owed", "2026-01-01T00:00:00Z");
    verdict_row(&conn, "ev-unowed", "2026-01-01T00:00:00Z");
    comemory::store::replica_outbox::enqueue(
        &conn,
        &NewOperation {
            operation_id: "op-owed",
            entity_kind: "feedback_event",
            entity_key: "ev-owed",
            op: ReplicaOp::Upsert,
            payload: Some(PayloadRef {
                digest: &owed,
                bytes: "unused: only the digest is read back",
            }),
            schema_version: 1,
            repository: Some("Falconiere/comemory"),
            origin: ReplicaOrigin::Local,
            at: NOW,
        },
        None,
    )
    .expect("enqueue owed push");

    let expired =
        replica_redaction::redact(&conn, replica_redaction::Reach::PastRetention(CUTOFF), NOW)
            .expect("expire");

    assert_eq!(
        expired,
        vec![unowed.clone()],
        "only the unowed digest expires"
    );
    assert_eq!(
        comemory::store::replica_redaction::redaction_of(&conn, &owed).expect("owed"),
        None,
        "the outbox still has to send this digest's real bytes"
    );
    assert_eq!(
        comemory::store::replica_redaction::redaction_of(&conn, &unowed).expect("unowed"),
        Some(Redaction::Expired)
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

/// Journal one payload of `kind` for `key` under `operation_id`, with `bytes`
/// as given, and return its digest.
fn journal_bytes(
    conn: &Connection,
    kind: &str,
    key: &str,
    operation_id: &str,
    bytes: &str,
) -> String {
    let digest = comemory::utilities::digest::sha256_hex(bytes.as_bytes());
    let epoch = stream_epoch(conn).expect("epoch");
    replica_journal::append(
        conn,
        &epoch,
        &NewOperation {
            operation_id,
            entity_kind: kind,
            entity_key: key,
            op: ReplicaOp::Upsert,
            payload: Some(PayloadRef {
                digest: &digest,
                bytes,
            }),
            schema_version: 1,
            repository: Some("Falconiere/comemory"),
            origin: ReplicaOrigin::Local,
            at: NOW,
        },
    )
    .expect("append");
    digest
}

#[test]
fn an_entity_erase_reaches_every_revision_of_it_and_nothing_else() {
    let (_dir, conn) = migrated_db();
    let first = journal_bytes(&conn, "memory", "a1b2c3d4", "op-1", r#"{"body":"first"}"#);
    let second = journal_bytes(&conn, "memory", "a1b2c3d4", "op-2", r#"{"body":"second"}"#);
    let other = journal_bytes(&conn, "memory", "e5f6a7b8", "op-3", r#"{"body":"other"}"#);
    let same_key_other_kind = journal_bytes(
        &conn,
        "document_revision",
        "a1b2c3d4",
        "op-4",
        r#"{"doc":1}"#,
    );
    // An expired copy is upgraded: the erase is the stronger claim.
    conn.execute(
        "UPDATE replica_payload SET bytes = NULL, redacted_at = ?2, redaction = 'expired' \
          WHERE digest = ?1",
        [&first, NOW],
    )
    .expect("expire the first revision");

    let mut erased = replica_redaction::redact(
        &conn,
        replica_redaction::Reach::Entity("memory", "a1b2c3d4"),
        NOW,
    )
    .expect("erase");
    erased.sort();
    let mut expected = vec![first.clone(), second.clone()];
    expected.sort();
    assert_eq!(erased, expected, "every revision the entity's feed names");
    for digest in [&first, &second] {
        assert_eq!(
            replica_redaction::redaction_of(&conn, digest).expect("redaction"),
            Some(Redaction::Erased)
        );
    }
    for untouched in [&other, &same_key_other_kind] {
        assert_eq!(
            replica_redaction::redaction_of(&conn, untouched).expect("redaction"),
            None,
            "another entity keeps its bytes"
        );
    }
    assert!(
        replica_redaction::redact(
            &conn,
            replica_redaction::Reach::Entity("memory", "a1b2c3d4"),
            NOW,
        )
        .expect("again")
        .is_empty(),
        "a second erase finds nothing left to blank"
    );
    assert_eq!(
        replica_read::page(&conn, 0, 10, None).expect("page").len(),
        4,
        "every position survives"
    );
}

#[test]
fn runs_naming_a_memory_are_erased_by_their_row_and_by_their_payload() {
    let (_dir, conn) = migrated_db();
    let by_row = journal_bytes(
        &conn,
        "activity_event",
        "ev-save",
        "op-save",
        r#"{"command":"save","summary":{"id":"a1b2c3d4"}}"#,
    );
    conn.execute(
        "INSERT INTO activity_log (at, command, source, duration_ms, ok, summary, event_id) \
         VALUES (?1, 'save', 'cli', 1, 1, '{\"id\":\"a1b2c3d4\",\"title\":null}', 'ev-save')",
        [NOW],
    )
    .expect("the save run's row");
    // A run evicted locally: no row left, only its journal copy names the id.
    let by_payload = journal_bytes(
        &conn,
        "activity_event",
        "ev-evicted",
        "op-evicted",
        r#"{"command":"delete","summary":{"id":"a1b2c3d4"}}"#,
    );
    let unrelated = journal_bytes(
        &conn,
        "activity_event",
        "ev-other",
        "op-other",
        r#"{"command":"save","summary":{"id":"e5f6a7b8"}}"#,
    );
    conn.execute(
        "INSERT INTO activity_log (at, command, source, duration_ms, ok, summary, event_id) \
         VALUES (?1, 'find', 'cli', 1, 1, 'not json', NULL)",
        [NOW],
    )
    .expect("a malformed summary the reach must skip, not fail on");

    let mut erased =
        replica_redaction::redact(&conn, replica_redaction::Reach::RunsNaming("a1b2c3d4"), NOW)
            .expect("erase");
    erased.sort();
    let mut expected = vec![by_row, by_payload];
    expected.sort();
    assert_eq!(erased, expected);
    assert_eq!(
        replica_redaction::redaction_of(&conn, &unrelated).expect("redaction"),
        None
    );
}
