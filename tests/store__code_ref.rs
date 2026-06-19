//! Real-DB coverage for the `code_ref` anchor store: upsert/for_memory
//! round-trip with anchors intact, full-replace semantics on re-upsert, and
//! `materialize` writing both the `edges` graph rows and the `code_ref`
//! anchor rows. Runs against a fully migrated `comemory.db` (migration 0009
//! creates the `code_ref` table) via the same `connection::open` path the
//! production code uses.

use comemory::memory::{Ref, References};
use comemory::store::{code_ref, connection};
use rusqlite::Connection;
use tempfile::tempdir;

const MEM: &str = "memabc12";

/// Open a fresh, fully migrated database in a temp dir.
fn open_db() -> (tempfile::TempDir, Connection) {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// A `References` with one anchored file ref and one anchored symbol ref.
fn anchored_refs() -> References {
    References {
        files: vec![Ref {
            id: "qwick:src/db.rs".to_string(),
            blob: Some("blobfile00".to_string()),
            commit: Some("commitfile".to_string()),
            branch: Some("main".to_string()),
        }],
        symbols: vec![Ref {
            id: "qwick:src/db.rs:connect".to_string(),
            blob: Some("blobsym000".to_string()),
            commit: Some("commitsym0".to_string()),
            branch: Some("main".to_string()),
        }],
    }
}

/// Count `code_ref` / `edges` rows matching a memory + rel filter.
fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [MEM], |r| r.get(0)).expect("count")
}

#[test]
fn upsert_then_for_memory_returns_rows_with_anchors() {
    let (_dir, conn) = open_db();
    code_ref::upsert(&conn, MEM, &anchored_refs(), "2026-06-19T00:00:00Z").expect("upsert");

    let rows = code_ref::for_memory(&conn, MEM).expect("for_memory");
    assert_eq!(rows.len(), 2, "one file + one symbol ref");

    // Ordered by (rel, dst_id): references_file sorts before references_symbol.
    let file = &rows[0];
    assert_eq!(file.rel, "references_file");
    assert_eq!(file.dst_id, "qwick:src/db.rs");
    assert_eq!(file.pinned_blob.as_deref(), Some("blobfile00"));
    assert_eq!(file.pinned_commit.as_deref(), Some("commitfile"));
    assert_eq!(file.branch.as_deref(), Some("main"));

    let sym = &rows[1];
    assert_eq!(sym.rel, "references_symbol");
    assert_eq!(sym.dst_id, "qwick:src/db.rs:connect");
    assert_eq!(sym.pinned_blob.as_deref(), Some("blobsym000"));
    assert_eq!(sym.pinned_commit.as_deref(), Some("commitsym0"));
}

#[test]
fn reupsert_with_smaller_set_drops_removed_ref() {
    let (_dir, conn) = open_db();
    code_ref::upsert(&conn, MEM, &anchored_refs(), "2026-06-19T00:00:00Z").expect("upsert");
    assert_eq!(code_ref::for_memory(&conn, MEM).expect("first").len(), 2);

    // Re-upsert with only the file ref: the symbol ref must be dropped, not
    // left behind (full-replace, unlike the additive edges table).
    let smaller = References {
        files: vec![Ref::new("qwick:src/db.rs")],
        symbols: vec![],
    };
    code_ref::upsert(&conn, MEM, &smaller, "2026-06-19T01:00:00Z").expect("re-upsert");

    let rows = code_ref::for_memory(&conn, MEM).expect("second");
    assert_eq!(rows.len(), 1, "removed symbol ref must be dropped");
    assert_eq!(rows[0].rel, "references_file");
    assert_eq!(rows[0].dst_id, "qwick:src/db.rs");
    // The surviving ref was re-supplied bare, so its anchor is now cleared.
    assert_eq!(rows[0].pinned_blob, None);
}

#[test]
fn materialize_writes_both_edges_and_code_ref() {
    let (_dir, conn) = open_db();
    code_ref::materialize(&conn, MEM, &anchored_refs(), "2026-06-19T00:00:00Z")
        .expect("materialize");

    // edges table: one references_file + one references_symbol row.
    let file_edges = count(
        &conn,
        "SELECT count(*) FROM edges WHERE src_kind='memory' AND src_id=?1 \
           AND rel='references_file' AND dst_kind='file' AND dst_id='qwick:src/db.rs'",
    );
    assert_eq!(file_edges, 1, "expected one references_file edge");

    let sym_edges = count(
        &conn,
        "SELECT count(*) FROM edges WHERE src_kind='memory' AND src_id=?1 \
           AND rel='references_symbol' AND dst_kind='symbol' \
           AND dst_id='qwick:src/db.rs:connect'",
    );
    assert_eq!(sym_edges, 1, "expected one references_symbol edge");

    // code_ref table: both anchor rows present.
    let anchor_rows = count(&conn, "SELECT count(*) FROM code_ref WHERE memory_id=?1");
    assert_eq!(anchor_rows, 2, "expected two code_ref anchor rows");

    let rows = code_ref::for_memory(&conn, MEM).expect("for_memory");
    assert_eq!(rows[0].pinned_blob.as_deref(), Some("blobfile00"));
    assert_eq!(rows[1].pinned_blob.as_deref(), Some("blobsym000"));
}
