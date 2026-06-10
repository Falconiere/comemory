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

/// `delete_outgoing` must remove only edges *originating* at the node:
/// incoming edges (e.g. a newer memory's `supersedes` pointing at it) have
/// to survive — `store::memory_row` relies on this when re-saving or
/// rebuilding a memory that something else supersedes.
#[test]
fn delete_outgoing_keeps_incoming_edges() {
    let conn = seed_db();
    for (src, dst) in [("old1", "tag-x"), ("new1", "old1")] {
        edges::insert(
            &conn,
            EdgeKey {
                src_kind: "memory",
                src_id: src,
                dst_kind: if dst == "tag-x" { "tag" } else { "memory" },
                dst_id: dst,
                rel: if dst == "tag-x" {
                    "tagged"
                } else {
                    "supersedes"
                },
            },
        )
        .expect("insert edge");
    }

    edges::delete_outgoing(&conn, "memory", "old1").expect("delete outgoing");

    // old1's own tagged edge is gone...
    assert!(edges::outgoing(&conn, "memory", "old1", "tagged")
        .expect("outgoing")
        .is_empty());
    // ...but the incoming supersedes edge from new1 survives.
    let incoming = edges::outgoing(&conn, "memory", "new1", "supersedes").expect("outgoing");
    assert_eq!(incoming, vec![("memory".to_string(), "old1".to_string())]);
}

/// A cyclic supersedes graph (a→b, b→a) must not loop forever. UNION in the
/// recursive CTE deduplicates (id, depth) tuples so the walk terminates at
/// max_depth even when back-edges exist.
#[test]
fn supersedes_chain_handles_cycle() {
    let conn = seed_db();
    for (src, dst) in [("a", "b"), ("b", "a")] {
        edges::insert(
            &conn,
            EdgeKey {
                src_kind: "memory",
                src_id: src,
                dst_kind: "memory",
                dst_id: dst,
                rel: "supersedes",
            },
        )
        .expect("insert cycle edge");
    }

    // Must return in finite time (not hang) and the result must be bounded.
    let chain = edges::supersedes_chain(&conn, "a", 20).expect("walk cyclic graph");
    // Both b and a (via b→a) may appear but the list must be short: at most
    // max_depth entries and must not grow exponentially.
    assert!(
        chain.len() <= 20,
        "cycle must not produce more results than max_depth; got {} entries",
        chain.len()
    );
    // 'b' must appear — it is the direct successor of 'a'.
    assert!(
        chain.contains(&"b".to_string()),
        "expected 'b' in the chain; got {chain:?}"
    );
}
