//! Verifies the shared `memory_row::insert` helper writes the `memories`
//! row, every `memory_tags` row, the `memory_fts` index entry, and the
//! v0.2 edges (in_repo / authored_by / tagged plus cross-link references)
//! that both `cli::save` and `cli::rebuild` depend on.

use comemory::memory::{Frontmatter, Kind, References, Relations};
use comemory::store::{connection, memory_row};
use rusqlite::Connection;
use tempfile::tempdir;
use time::OffsetDateTime;

const ID: &str = "abc12345";

fn sample_fm() -> Frontmatter {
    Frontmatter {
        id: ID.to_string(),
        kind: Kind::Decision,
        repo: "qwick".to_string(),
        tags: vec!["db".to_string(), "postgres".to_string()],
        author: "alice".to_string(),
        created: OffsetDateTime::now_utc(),
        quality: 4,
        schema: 1,
        content_hash: "deadbeef".to_string(),
        references: References::default(),
        relations: Relations::default(),
    }
}

fn count_by_id(conn: &Connection, table: &str, col: &str) -> i64 {
    let sql = format!("SELECT count(*) FROM {table} WHERE {col} = ?1");
    conn.query_row(&sql, [ID], |r| r.get(0)).expect("count")
}

fn assert_edge(conn: &Connection, rel: &str, dst_kind: &str, dst_id: &str) {
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM edges WHERE src_kind = 'memory' AND src_id = ?1 \
               AND rel = ?2 AND dst_kind = ?3 AND dst_id = ?4",
            rusqlite::params![ID, rel, dst_kind, dst_id],
            |r| r.get(0),
        )
        .expect("count edges");
    assert_eq!(n, 1, "expected edge {rel} -> {dst_kind}:{dst_id}");
}

fn assert_row_counts(conn: &Connection) {
    assert_eq!(count_by_id(conn, "memories", "id"), 1);
    assert_eq!(count_by_id(conn, "memory_tags", "memory_id"), 2);
    assert_eq!(count_by_id(conn, "memory_fts", "memory_id"), 1);
}

fn assert_all_edges(conn: &Connection) {
    assert_edge(conn, "in_repo", "repo", "qwick");
    assert_edge(conn, "authored_by", "author", "alice");
    assert_edge(conn, "tagged", "tag", "db");
    assert_edge(conn, "tagged", "tag", "postgres");
    assert_edge(conn, "references_file", "file", "qwick:src/lib.rs");
    assert_edge(
        conn,
        "references_symbol",
        "symbol",
        "qwick:src/lib.rs:start",
    );
}

/// Run `memory_row::insert` for `body` inside its own transaction.
fn insert_body(conn: &mut Connection, fm: &Frontmatter, body: &str) {
    let tx = conn.transaction().expect("tx");
    memory_row::insert(&tx, fm, body, "slug-x", "/abs/path.md", &fm.tags).expect("insert");
    tx.commit().expect("commit");
}

/// Read the persisted `memories.simhash` for the fixture id.
fn stored_simhash(conn: &Connection) -> i64 {
    conn.query_row("SELECT simhash FROM memories WHERE id = ?1", [ID], |r| {
        r.get(0)
    })
    .expect("simhash row")
}

#[test]
fn insert_persists_simhash_and_upsert_refreshes_it() {
    let dir = tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let fm = sample_fm();

    let body_a = "advisory locks serialize concurrent migrations in postgres";
    insert_body(&mut conn, &fm, body_a);
    let expected_a = comemory::simhash::simhash64(comemory::simhash::tokens(body_a)) as i64;
    let got_a = stored_simhash(&conn);
    assert_ne!(
        got_a, 0,
        "fresh insert must not leave the DEFAULT 0 simhash"
    );
    assert_eq!(
        got_a, expected_a,
        "stored simhash != simhash64(tokens(body))"
    );

    // Re-save with the same id but a changed body must hit the ON CONFLICT
    // upsert arm and refresh the fingerprint, not keep the stale one.
    let body_b = "completely different note about ast-grep pattern syntax";
    insert_body(&mut conn, &fm, body_b);
    let expected_b = comemory::simhash::simhash64(comemory::simhash::tokens(body_b)) as i64;
    let got_b = stored_simhash(&conn);
    assert_eq!(got_b, expected_b, "upsert must refresh simhash");
    assert_ne!(got_b, got_a, "changed body should change the simhash");
}

#[test]
fn frontmatter_relations_materialize_as_memory_edges() {
    let dir = tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let mut fm = sample_fm();
    fm.relations = Relations {
        supersedes: vec!["11111111".to_string()],
        conflicts_with: vec!["22222222".to_string()],
        derived_from: vec!["33333333".to_string()],
    };

    insert_body(&mut conn, &fm, "newer convention body");

    // Direction: src = the memory carrying the relation, dst = the target.
    // Targets are dangling (no `memories` row exists for them) — tolerated
    // by design, same as cross-link refs.
    assert_edge(&conn, "supersedes", "memory", "11111111");
    assert_edge(&conn, "conflicts_with", "memory", "22222222");
    assert_edge(&conn, "derived_from", "memory", "33333333");

    // Re-insert (upsert path) must stay idempotent: still exactly one edge
    // per relation, not duplicates.
    insert_body(&mut conn, &fm, "newer convention body");
    assert_edge(&conn, "supersedes", "memory", "11111111");
    assert_edge(&conn, "conflicts_with", "memory", "22222222");
    assert_edge(&conn, "derived_from", "memory", "33333333");
}

#[test]
fn inserts_row_tags_fts_and_edges() {
    let dir = tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let tx = conn.transaction().expect("tx");
    let fm = sample_fm();
    let body = "use `qwick:src/lib.rs:start` for bootstrap";
    memory_row::insert(&tx, &fm, body, "slug-x", "/abs/path.md", &fm.tags).expect("insert");
    tx.commit().expect("commit");

    assert_row_counts(&conn);
    assert_all_edges(&conn);
}
