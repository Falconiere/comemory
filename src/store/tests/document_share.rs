#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `document_share` against a real migrated database: the mapping a local
//! document keeps, the collision it must refuse rather than merge, and the
//! cascade that ties its lifetime to the document it names.
//!
//! Every case seeds the real `source_roots` -> `source_files` -> `documents`
//! chain first. That is not ceremony: `document_share.document_id` is
//! `REFERENCES documents(id)`, so a share row over an invented id is not a
//! simplification, it is a row SQLite refuses.

use comemory::store::document_share::{self, Share};
use comemory::store::documents::{self, DocumentUpsert};
use comemory::store::sources::{self, SourceFileUpsert, SourceRootUpsert};
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

/// Seed the FK chain above one `documents` row, so `document_id` resolves.
/// Each document gets its own source root: `source_roots.canonical_path` and
/// `documents.source_file_id` are both `UNIQUE`, so two documents cannot share
/// either parent.
fn seed_document(conn: &Connection, document_id: &str, relative_path: &str) {
    sources::upsert(
        conn,
        SourceRootUpsert {
            id: &format!("source-of-{document_id}"),
            canonical_path: &format!("/does/not/matter/{document_id}"),
            kind: "dir",
            repo: Some(REPO),
            created_at: AT,
            updated_at: AT,
        },
    )
    .expect("seed source_roots row");
    sources::upsert_file(
        conn,
        SourceFileUpsert {
            id: &format!("file-of-{document_id}"),
            source_id: &format!("source-of-{document_id}"),
            relative_path,
            classification: "document",
            size: 0,
            mtime: 0,
            sha256: None,
            status: "indexed",
            error: None,
            created_at: AT,
            updated_at: AT,
        },
    )
    .expect("seed source_files row");
    documents::upsert_document(
        conn,
        DocumentUpsert {
            id: document_id,
            source_file_id: &format!("file-of-{document_id}"),
            title: "Cloud sync",
            repo: Some(REPO),
            revision_hash: "hash",
            created_at: AT,
            updated_at: AT,
        },
    )
    .expect("seed documents row");
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
    seed_document(&conn, "local-doc-1", "docs/guides/cloud-sync.md");
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
    seed_document(&conn, "local-doc-1", "docs/guides/cloud-sync.md");
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
    seed_document(&conn, "local-doc-1", "docs/README.md");
    seed_document(&conn, "local-doc-2", "docs/readme.md");
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
    seed_document(&conn, "local-doc-1", "docs/guides/cloud-sync.md");
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
fn deleting_the_document_takes_its_portable_name_with_it() {
    let (_dir, conn) = migrated_db();
    seed_document(&conn, "local-doc-1", "docs/guides/cloud-sync.md");
    document_share::record(
        &conn,
        &share("local-doc-1", "docs/guides/cloud-sync.md", &"a".repeat(32)),
        AT,
    )
    .expect("record");

    // The production deletion path, not a purge written for this table:
    // `document::writer::tombstone` and `unindex` both remove the parent row,
    // and `ON DELETE CASCADE` is what carries the share away with it.
    documents::delete_document(&conn, "local-doc-1").expect("delete the document");

    assert_eq!(
        document_share::by_document(&conn, "local-doc-1").expect("by_document"),
        None,
        "no deletion path has to remember this table"
    );
    // The freed name is claimable, so a successor document at that path gets it.
    seed_document(&conn, "local-doc-2", "docs/guides/cloud-sync.md");
    let successor = share("local-doc-2", "docs/guides/cloud-sync.md", &"a".repeat(32));
    document_share::record(&conn, &successor, AT).expect("the name was released");
}

#[test]
fn a_name_for_a_document_that_does_not_exist_is_refused() {
    let (_dir, conn) = migrated_db();

    // No `documents` row was seeded. If the foreign key were missing this
    // would succeed and leave a share naming nothing — the orphan the cascade
    // exists to make impossible.
    let refused = document_share::record(
        &conn,
        &share("no-such-doc", "docs/guides/cloud-sync.md", &"a".repeat(32)),
        AT,
    )
    .expect_err("the foreign key must reject it");

    assert!(
        matches!(&refused, comemory::errors::Error::NotFound(m)
            if m.contains("no-such-doc")),
        "a missing parent is its own problem, not a name collision: got {refused:?}"
    );
}

#[test]
fn the_same_name_under_another_repository_is_a_different_row() {
    let (_dir, conn) = migrated_db();
    seed_document(&conn, "local-doc-1", "docs/README.md");
    seed_document(&conn, "local-doc-2", "docs/README.md");
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
    seed_document(&conn, "local-doc-1", "docs/README.md");
    seed_document(&conn, "local-doc-2", "docs/readme.md");
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
                if inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
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
