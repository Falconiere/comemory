#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `document_share` against a real migrated database: the mapping a local
//! document keeps, and the collision it must refuse rather than merge.

use comemory::store::document_share::{self, Share};
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

const REPO: &str = "Falconiere/comemory";
const AT: &str = "2026-09-23T10:00:00Z";

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

fn share(document_id: &str, path: &str, shared_id: &str) -> Share {
    Share {
        document_id: document_id.to_string(),
        repo: REPO.to_string(),
        shared_id: shared_id.to_string(),
        path: path.to_string(),
        blocked_reason: None,
    }
}

#[test]
fn a_local_document_keeps_its_own_id_beside_the_portable_one() {
    let (_dir, conn) = migrated_db();
    let row = share("local-doc-1", "docs/guides/cloud-sync.md", &"a".repeat(32));

    document_share::record(&conn, &row, AT).expect("record");

    let read = document_share::by_document(&conn, "local-doc-1")
        .expect("by_document")
        .expect("row");
    assert_eq!(read, row, "the local id is the key and is not rewritten");
    assert_eq!(
        document_share::by_shared_id(&conn, REPO, &"a".repeat(32))
            .expect("by_shared_id")
            .expect("row")
            .document_id,
        "local-doc-1",
        "and the portable name resolves back to it"
    );
}

#[test]
fn re_recording_the_same_document_refreshes_rather_than_conflicts() {
    let (_dir, conn) = migrated_db();
    let first = share("local-doc-1", "docs/guides/cloud-sync.md", &"a".repeat(32));
    document_share::record(&conn, &first, AT).expect("first");

    // The document moved: same local id, new portable name.
    let moved = share("local-doc-1", "docs/cloud-sync.md", &"b".repeat(32));
    document_share::record(&conn, &moved, "2026-09-23T11:00:00Z").expect("refresh");

    assert_eq!(
        document_share::by_document(&conn, "local-doc-1")
            .expect("by_document")
            .expect("row"),
        moved
    );
    assert_eq!(
        document_share::by_shared_id(&conn, REPO, &"a".repeat(32)).expect("by_shared_id"),
        None,
        "the name it no longer holds is free again"
    );
}

#[test]
fn two_documents_normalizing_onto_one_name_is_refused() {
    let (_dir, conn) = migrated_db();
    let held = share("local-doc-1", "docs/README.md", &"a".repeat(32));
    document_share::record(&conn, &held, AT).expect("first");

    // A different local document — `docs/readme.md` on a case-insensitive
    // filesystem — computing the same portable name.
    let collides = share("local-doc-2", "docs/readme.md", &"a".repeat(32));
    let refused = document_share::record(&conn, &collides, AT)
        .expect_err("two unrelated files may not share one name");

    assert!(
        matches!(&refused, comemory::errors::Error::Conflict(m)
            if m.contains("local-doc-1") && m.contains("docs/readme.md")),
        "got {refused:?}"
    );
    // Both local documents survive the refusal: nothing was merged.
    assert_eq!(
        document_share::by_document(&conn, "local-doc-1")
            .expect("by_document")
            .expect("row"),
        held,
        "the holder is untouched"
    );
    assert_eq!(
        document_share::by_document(&conn, "local-doc-2").expect("by_document"),
        None,
        "and the loser simply has no portable name"
    );
}

#[test]
fn a_blocked_document_records_why() {
    let (_dir, conn) = migrated_db();
    let mut row = share("local-doc-1", "docs/guides/cloud-sync.md", &"a".repeat(32));
    row.blocked_reason = Some("aws access key id".to_string());

    document_share::record(&conn, &row, AT).expect("record");

    assert_eq!(
        document_share::by_document(&conn, "local-doc-1")
            .expect("by_document")
            .expect("row")
            .blocked_reason
            .as_deref(),
        Some("aws access key id"),
        "the reason an operator has to be able to see"
    );
}

#[test]
fn forgetting_a_document_frees_its_name() {
    let (_dir, conn) = migrated_db();
    let row = share("local-doc-1", "docs/guides/cloud-sync.md", &"a".repeat(32));
    document_share::record(&conn, &row, AT).expect("record");

    document_share::forget(&conn, "local-doc-1").expect("forget");

    assert_eq!(
        document_share::by_document(&conn, "local-doc-1").expect("by_document"),
        None
    );
    // The name is reusable, so a later document at that path can claim it.
    let successor = share("local-doc-2", "docs/guides/cloud-sync.md", &"a".repeat(32));
    document_share::record(&conn, &successor, AT).expect("the name was released");
}

#[test]
fn the_same_name_under_another_repository_is_a_different_row() {
    let (_dir, conn) = migrated_db();
    document_share::record(
        &conn,
        &share("local-doc-1", "docs/README.md", &"a".repeat(32)),
        AT,
    )
    .expect("first");

    let other_repo = Share {
        repo: "Falconiere/other".to_string(),
        ..share("local-doc-2", "docs/README.md", &"a".repeat(32))
    };
    document_share::record(&conn, &other_repo, AT).expect("a different repo does not collide");

    assert_eq!(
        document_share::by_shared_id(&conn, "Falconiere/other", &"a".repeat(32))
            .expect("by_shared_id")
            .expect("row")
            .document_id,
        "local-doc-2"
    );
}

#[test]
fn the_unique_index_refuses_a_collision_that_bypasses_record() {
    let (_dir, conn) = migrated_db();
    document_share::record(
        &conn,
        &share("local-doc-1", "docs/README.md", &"a".repeat(32)),
        AT,
    )
    .expect("first");

    // Straight past `record`'s read, the way a racing writer would arrive. If
    // the constraint were missing this INSERT would succeed, and two unrelated
    // files would quietly share one name.
    let raced = conn.execute(
        "INSERT INTO document_share(document_id, repo, shared_id, path, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params!["local-doc-2", REPO, "a".repeat(32), "docs/readme.md", AT],
    );

    let error = raced.expect_err("uq_document_share_shared must reject it");
    assert!(
        matches!(
            &error,
            rusqlite::Error::SqliteFailure(inner, _)
                if inner.code == rusqlite::ErrorCode::ConstraintViolation
        ),
        "got {error:?}"
    );
    assert_eq!(
        document_share::by_shared_id(&conn, REPO, &"a".repeat(32))
            .expect("by_shared_id")
            .expect("row")
            .document_id,
        "local-doc-1",
        "the holder still holds it"
    );
}
