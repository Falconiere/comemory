#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/prune_signals.rs` — the low-quality/degree
//! scan and the superseded-and-forgotten scan behind `prune::low_value`.
//! Pins the `<=`/`<` boundaries directly: `prune --apply` deletes whatever
//! these two queries return, so an off-by-one here is a data-loss bug.

use comemory::store::connection;
use comemory::store::prune_signals::{quality_and_degree_candidates, superseded_and_forgotten};

fn seed_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn seed_memory(conn: &rusqlite::Connection, id: &str, quality: u8, last_accessed: &str) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, access_count, last_accessed, simhash)
         VALUES (?1, ?1, 'note', 'demo', 'f', ?2, 1, 'h-' || ?1, 'body ' || ?1,
                 '2024-12-01T00:00:00Z', '2024-12-01T00:00:00Z', 'm/' || ?1 || '.md', 0, ?3, 0)",
        rusqlite::params![id, quality, last_accessed],
    )
    .expect("seed memory");
}

#[test]
fn quality_boundary_is_inclusive() {
    let (_d, conn) = seed_db();
    // At the threshold (2): included. One above (3): excluded.
    seed_memory(&conn, "aaaa0001", 2, "2025-01-01T00:00:00Z");
    seed_memory(&conn, "aaaa0002", 3, "2025-01-01T00:00:00Z");

    let rows = quality_and_degree_candidates(&conn, 2).expect("query");
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["aaaa0001"],
        "quality <= 2 must be inclusive at the boundary"
    );
}

#[test]
fn incoming_edge_excludes_a_candidate() {
    let (_d, conn) = seed_db();
    seed_memory(&conn, "aaaa0001", 2, "2025-01-01T00:00:00Z");
    seed_memory(&conn, "aaaa0002", 2, "2025-01-01T00:00:00Z");
    conn.execute(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0002','memory','aaaa0001','derived_from','2026-01-01T00:00:00Z')",
        [],
    )
    .expect("seed edge");

    let rows = quality_and_degree_candidates(&conn, 2).expect("query");
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["aaaa0002"],
        "a memory with an incoming edge must be excluded"
    );
}

#[test]
fn superseded_cutoff_boundary_is_exclusive() {
    let (_d, conn) = seed_db();
    // Superseded-and-never-accessed-since shape: edge exactly AT the cutoff
    // must NOT match (`e.created_at < cutoff`, strict); a cutoff one second
    // later must match the same edge.
    seed_memory(&conn, "aaaa0001", 4, "2025-06-01T00:00:00Z");
    seed_memory(&conn, "aaaa0002", 4, "2026-06-01T00:00:00Z");
    conn.execute(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0002','memory','aaaa0001','supersedes','2026-01-01T00:00:00Z')",
        [],
    )
    .expect("seed edge");

    let cutoff = "2026-01-01T00:00:00Z";
    let ids = superseded_and_forgotten(&conn, cutoff).expect("query at exact cutoff");
    assert!(
        !ids.contains(&"aaaa0001".to_string()),
        "an edge exactly AT the cutoff must not match (strict `<`): {ids:?}"
    );

    let cutoff_after = "2026-01-01T00:00:01Z";
    let ids = superseded_and_forgotten(&conn, cutoff_after).expect("query one second later");
    assert!(
        ids.contains(&"aaaa0001".to_string()),
        "an edge one second before the cutoff must match: {ids:?}"
    );
}
