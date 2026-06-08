use comemory::graph::edges::{self, EdgeKey};
use comemory::store::connection;
use tempfile::tempdir;

fn seed_db() -> rusqlite::Connection {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    connection::open(&path).expect("open")
}

#[test]
fn insert_edge_then_neighbors_returns_it() {
    let conn = seed_db();
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "memory",
            src_id: "abcd1234",
            dst_kind: "memory",
            dst_id: "efgh5678",
            rel: "supersedes",
        },
    )
    .expect("insert");

    let nbrs = edges::outgoing(&conn, "memory", "abcd1234", "supersedes").expect("outgoing");
    assert_eq!(nbrs.len(), 1);
    assert_eq!(nbrs[0], ("memory".to_string(), "efgh5678".to_string()));
}

#[test]
fn supersedes_walk_is_transitive() {
    let conn = seed_db();
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "memory",
            src_id: "a",
            dst_kind: "memory",
            dst_id: "b",
            rel: "supersedes",
        },
    )
    .expect("insert a→b");
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "memory",
            src_id: "b",
            dst_kind: "memory",
            dst_id: "c",
            rel: "supersedes",
        },
    )
    .expect("insert b→c");

    let chain = edges::supersedes_chain(&conn, "a", 5).expect("walk");
    assert_eq!(chain, vec!["b".to_string(), "c".to_string()]);
}
