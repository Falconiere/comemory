#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`comemory::store::replica_sweep`] against a real migrated database: what
//! an abandoned upload leaves behind, and everything the sweep must walk past
//! — an active generation, its projection, and every receipt.

use comemory::store::code_generation::{self, Generation, State};
use comemory::store::remote_code::{Edge, File, Projection};
use comemory::store::replica_journal::ReplicaOrigin;
use comemory::store::replica_receipt::{self, Receipt};
use comemory::store::{connection, migrate, remote_code, replica_staging, replica_sweep};
use rusqlite::Connection;
use tempfile::TempDir;
use time::OffsetDateTime;
use time::macros::datetime;

const REPO: &str = "Falconiere/comemory";

/// The instant every case sweeps at.
const NOW: OffsetDateTime = datetime!(2026-09-22 12:00:00 UTC);

/// Two days before [`NOW`] — past the abandonment window.
const LONG_AGO: &str = "2026-09-20T12:00:00Z";

/// One hour before [`NOW`] — a live upload.
const RECENTLY: &str = "2026-09-22T11:00:00Z";

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

fn projection() -> Projection {
    Projection {
        files: vec![File {
            path: "src/lib.rs".to_string(),
            blob_oid: "aaaa1111".to_string(),
        }],
        symbols: Vec::new(),
        edges: vec![Edge {
            rel: "imports".to_string(),
            src_path: "src/lib.rs".to_string(),
            dst_path: "src/store.rs".to_string(),
            weight: 1,
            anchor: Some("aaaa1111".to_string()),
        }],
    }
}

/// Record one generation with its projection, leaving it `staged`.
fn stage(conn: &Connection, generation_id: &str, at: &str) {
    code_generation::record(
        conn,
        &Generation {
            repo: REPO.to_string(),
            generation_id: generation_id.to_string(),
            parent_id: None,
            head: "head-1".to_string(),
            mined_commit: None,
            origin: ReplicaOrigin::Sync,
            state: State::Staged,
            file_count: 1,
            manifest_digest: "d".repeat(64),
        },
        at,
    )
    .expect("record");
    remote_code::replace_generation(conn, REPO, generation_id, &projection()).expect("projection");
}

/// Record, project and activate one generation.
fn activate(conn: &Connection, generation_id: &str, at: &str) {
    stage(conn, generation_id, at);
    code_generation::activate(conn, REPO, generation_id, at).expect("activate");
}

/// One receipt, as an acceptance writes it.
fn receipt(conn: &Connection, operation_id: &str, at: &str) {
    replica_receipt::record(
        conn,
        &Receipt {
            operation_id: operation_id.to_string(),
            epoch: "epoch-1".to_string(),
            sequence: Some(1),
            disposition: "accepted".to_string(),
            payload_digest: Some("c".repeat(64)),
            reason: None,
        },
        at,
    )
    .expect("receipt");
}

#[test]
fn a_sweep_spares_the_active_generation_its_projection_and_every_receipt() {
    let (_dir, mut conn) = migrated_db();
    activate(&conn, &"a".repeat(32), LONG_AGO);
    receipt(&conn, "op-20260920-gen00001", LONG_AGO);
    // A second upload that never activated, from the same day.
    stage(&conn, &"b".repeat(32), LONG_AGO);
    replica_staging::put_part(&conn, "upload-abandoned", 0, 2, "{\"half\":1}", LONG_AGO)
        .expect("part");

    let swept = replica_sweep::run(&mut conn, NOW).expect("sweep");

    assert_eq!(swept.parts, 1);
    assert_eq!(swept.generations, 1);
    let active = code_generation::active(&conn, REPO)
        .expect("active")
        .expect("row");
    assert_eq!(active.generation_id, "a".repeat(32));
    assert_eq!(
        remote_code::projection(&conn, REPO, &"a".repeat(32))
            .expect("projection")
            .files
            .len(),
        1,
        "the active generation's projection is untouched"
    );
    assert!(
        replica_receipt::lookup(&conn, "op-20260920-gen00001")
            .expect("lookup")
            .is_some(),
        "and so is the receipt a peer's retry reads back"
    );
    assert_eq!(
        code_generation::by_id(&conn, REPO, &"b".repeat(32)).expect("by_id"),
        None,
        "the abandoned generation is gone"
    );
    assert!(
        remote_code::projection(&conn, REPO, &"b".repeat(32))
            .expect("projection")
            .files
            .is_empty(),
        "with the projection it owned"
    );
    assert_eq!(
        replica_staging::assemble(&conn, "upload-abandoned").expect("assemble"),
        None
    );
}

#[test]
fn an_upload_still_inside_its_window_is_left_alone() {
    let (_dir, mut conn) = migrated_db();
    stage(&conn, &"b".repeat(32), RECENTLY);
    replica_staging::put_part(&conn, "upload-live", 0, 2, "{\"half\":1}", RECENTLY).expect("part");

    let swept = replica_sweep::run(&mut conn, NOW).expect("sweep");

    assert_eq!(swept, replica_sweep::Swept::default());
    assert!(
        code_generation::by_id(&conn, REPO, &"b".repeat(32))
            .expect("by_id")
            .is_some(),
        "a sender retrying a part has not abandoned anything"
    );
    let state = replica_staging::put_part(&conn, "upload-live", 1, 2, "{\"half\":2}", RECENTLY)
        .expect("second part");
    assert!(state.complete(), "and its first part is still there");
}

#[test]
fn a_sweep_over_a_database_with_nothing_staged_changes_nothing() {
    let (_dir, mut conn) = migrated_db();
    activate(&conn, &"a".repeat(32), LONG_AGO);
    receipt(&conn, "op-20260920-gen00001", LONG_AGO);

    assert_eq!(
        replica_sweep::run(&mut conn, NOW).expect("sweep"),
        replica_sweep::Swept::default()
    );
    assert_eq!(
        replica_sweep::run(&mut conn, NOW).expect("sweep again"),
        replica_sweep::Swept::default(),
        "and it stays a no-op"
    );
    assert!(
        code_generation::active(&conn, REPO)
            .expect("active")
            .is_some()
    );
    assert!(
        replica_receipt::lookup(&conn, "op-20260920-gen00001")
            .expect("lookup")
            .is_some()
    );
}

#[test]
fn a_superseded_generation_is_not_debris() {
    let (_dir, mut conn) = migrated_db();
    activate(&conn, &"a".repeat(32), LONG_AGO);
    code_generation::record(
        &conn,
        &Generation {
            repo: REPO.to_string(),
            generation_id: "c".repeat(32),
            parent_id: Some("a".repeat(32)),
            head: "head-2".to_string(),
            mined_commit: None,
            origin: ReplicaOrigin::Sync,
            state: State::Staged,
            file_count: 1,
            manifest_digest: "d".repeat(64),
        },
        LONG_AGO,
    )
    .expect("record");
    code_generation::activate(&conn, REPO, &"c".repeat(32), LONG_AGO).expect("activate");

    let swept = replica_sweep::run(&mut conn, NOW).expect("sweep");

    assert_eq!(swept, replica_sweep::Swept::default());
    assert_eq!(
        code_generation::by_id(&conn, REPO, &"a".repeat(32))
            .expect("by_id")
            .expect("row")
            .state,
        State::Superseded,
        "history is not debris: only a generation that never activated is"
    );
}

#[test]
fn pending_generation_still_owed_upstream_survives_the_sweep() {
    let (_dir, mut conn) = migrated_db();
    stage(&conn, &"e".repeat(32), LONG_AGO);
    // The push that will deliver it is still queued — a client's generation
    // is staged until the upstream accepts it, however long backoff takes.
    let tx = conn.transaction().expect("tx");
    comemory::store::replica_outbox::enqueue(
        &tx,
        &comemory::store::replica_journal::NewOperation {
            operation_id: "op-20260920-owedgeneration000000000000001",
            entity_kind: "code_generation",
            entity_key: REPO,
            op: comemory::store::replica_journal::ReplicaOp::Upsert,
            payload: None,
            schema_version: 1,
            repository: Some(REPO),
            origin: ReplicaOrigin::Local,
            at: LONG_AGO,
        },
        None,
    )
    .expect("owed upload");
    tx.commit().expect("commit");

    let swept = replica_sweep::run(&mut conn, NOW).expect("sweep");

    assert_eq!(swept.generations, 0, "an owed generation is not abandoned");
    assert!(
        code_generation::by_id(&conn, REPO, &"e".repeat(32))
            .expect("by_id")
            .is_some()
    );
}
