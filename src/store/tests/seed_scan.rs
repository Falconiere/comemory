#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The scan journal seeding resumes from, against a real migrated database:
//! ordered, bounded, exclusive of the resume point, and blind to deletions.

use comemory::store::seed_scan;
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

/// Insert a mirror row the way a save would, minus the parts the scan does
/// not read.
fn insert_memory(conn: &Connection, id: &str, deleted: bool) {
    conn.execute(
        "INSERT INTO memories(\
             id, slug, kind, repo, author, quality, schema, content_hash, body, md_path, \
             created_at, updated_at, deleted_at) \
         VALUES(?1, 'slug', 'note', 'demo', '', 3, 1, ?2, 'body', 'memories/x.md', \
             '2026-09-21T10:00:00Z', '2026-09-21T10:00:00Z', ?3)",
        rusqlite::params![
            id,
            format!("{id}{id}"),
            deleted.then_some("2026-09-21T11:00:00Z")
        ],
    )
    .expect("insert memory");
}

#[test]
fn an_empty_corpus_has_nothing_to_seed() {
    let (_dir, conn) = migrated_db();
    assert!(
        seed_scan::live_memories_after(&conn, "", 100)
            .expect("scan")
            .is_empty()
    );
}

#[test]
fn the_scan_is_ordered_bounded_and_resumes_above_its_last_id() {
    let (_dir, conn) = migrated_db();
    for id in ["aaaa0003", "aaaa0001", "aaaa0002"] {
        insert_memory(&conn, id, false);
    }

    assert_eq!(
        seed_scan::live_memories_after(&conn, "", 10).expect("scan"),
        vec![
            "aaaa0001".to_string(),
            "aaaa0002".to_string(),
            "aaaa0003".to_string()
        ],
        "insertion order is not the resume order; id order is"
    );
    assert_eq!(
        seed_scan::live_memories_after(&conn, "", 2).expect("scan"),
        vec!["aaaa0001".to_string(), "aaaa0002".to_string()],
        "the batch is bounded"
    );
    assert_eq!(
        seed_scan::live_memories_after(&conn, "aaaa0002", 10).expect("scan"),
        vec!["aaaa0003".to_string()],
        "the resume point itself is not seeded twice"
    );
    assert!(
        seed_scan::live_memories_after(&conn, "aaaa0003", 10)
            .expect("scan")
            .is_empty(),
        "the tail of the scan is how seeding knows it is finished"
    );
}

#[test]
fn a_soft_deleted_memory_is_not_seeded() {
    let (_dir, conn) = migrated_db();
    insert_memory(&conn, "aaaa0001", false);
    insert_memory(&conn, "aaaa0002", true);

    assert_eq!(
        seed_scan::live_memories_after(&conn, "", 10).expect("scan"),
        vec!["aaaa0001".to_string()],
        "a deleted memory has nothing to replicate"
    );
}
