//! Tests for [`comemory::retrieval::bundle::assemble`].
//!
//! Covers: empty bundle, single memory pull, multi-rel depth-2 edge walk
//! (references_symbol, relates_to, supersedes).

use comemory::retrieval::bundle;
use comemory::store::connection;
use tempfile::tempdir;

#[test]
fn assemble_returns_empty_bundle_when_no_ids() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    let b = bundle::assemble(&conn, "advisory lock", &[]).expect("assemble");
    assert_eq!(b.query, "advisory lock");
    assert!(b.memories.is_empty());
    assert!(b.code_refs.is_empty());
    assert!(b.relations.is_empty());
}

#[test]
fn assemble_pulls_memory_rows_by_id() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('m1','s','decision','h','Use Postgres for analytics','t','t','x.md')",
        [],
    )
    .expect("seed");

    let b = bundle::assemble(&conn, "postgres", &["m1".to_string()]).expect("assemble");
    assert_eq!(b.memories.len(), 1);
    assert_eq!(b.memories[0].id, "m1");
    assert_eq!(b.memories[0].kind, "decision");
    assert_eq!(b.memories[0].body, "Use Postgres for analytics");

    let v: serde_json::Value = serde_json::to_value(&b).expect("json");
    assert_eq!(v["query"], "postgres");
    assert_eq!(v["memories"][0]["id"], "m1");
}

#[test]
fn assemble_walks_supersedes_chain_to_depth_2() {
    // m1 supersedes m2, m2 supersedes m3 — depth-2 walk from m1 must
    // surface both edges.
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    for (id, slug) in [("m1", "s1"), ("m2", "s2"), ("m3", "s3")] {
        conn.execute(
            "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
             VALUES(?1,?2,'note','h','body','t','t','p.md')",
            rusqlite::params![id, slug],
        )
        .expect("seed");
    }
    for (src, dst) in [("m1", "m2"), ("m2", "m3")] {
        conn.execute(
            "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
             VALUES('memory',?1,'memory',?2,'supersedes','t')",
            rusqlite::params![src, dst],
        )
        .expect("edge");
    }

    let b = bundle::assemble(&conn, "q", &["m1".to_string()]).expect("assemble");
    // Both hops must appear in relations.
    let rels: Vec<&str> = b.relations.iter().map(|r| r.rel.as_str()).collect();
    let supersedes_count = rels.iter().filter(|&&r| r == "supersedes").count();
    assert_eq!(
        supersedes_count,
        2,
        "expected 2 supersedes edges, got relations: {b:?}",
        b = b
            .relations
            .iter()
            .map(|r| format!("{}->{}", r.from, r.to))
            .collect::<Vec<_>>()
    );
}

#[test]
fn assemble_walks_relates_to_edges() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    for (id, slug) in [("m1", "s1"), ("m2", "s2")] {
        conn.execute(
            "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
             VALUES(?1,?2,'note','h','body','t','t','p.md')",
            rusqlite::params![id, slug],
        )
        .expect("seed");
    }
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES('memory','m1','memory','m2','relates_to','t')",
        [],
    )
    .expect("edge");

    let b = bundle::assemble(&conn, "q", &["m1".to_string()]).expect("assemble");
    assert!(
        b.relations.iter().any(|r| r.rel == "relates_to"),
        "relates_to edge missing from bundle relations: {rels:?}",
        rels = b.relations.iter().map(|r| &r.rel).collect::<Vec<_>>()
    );
}
