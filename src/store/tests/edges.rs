#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/store/edges.rs` — edge upserts and recursive-CTE
//! walks over the `edges` table.

use comemory::store::connection;
use comemory::store::edges::{self, EdgeKey};
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

/// `insert_weighted` is the accumulating writer the co-change post-pass
/// uses: a fresh edge lands with the given weight, and re-inserting the
/// same key ADDS to the stored weight instead of ignoring the row (each
/// mining run walks only commits newer than the cursor, so weights are
/// deltas, not totals).
#[test]
fn insert_weighted_accumulates_on_conflict() {
    let conn = seed_db();
    let key = EdgeKey {
        src_kind: "file",
        src_id: "file:r:a.rs",
        dst_kind: "file",
        dst_id: "file:r:b.rs",
        rel: "co_changed",
    };
    edges::insert_weighted(&conn, key, 2).expect("first insert");
    edges::insert_weighted(&conn, key, 3).expect("accumulating insert");

    let weight: i64 = conn
        .query_row(
            "SELECT weight FROM edges WHERE src_id='file:r:a.rs' \
             AND dst_id='file:r:b.rs' AND rel='co_changed'",
            [],
            |r| r.get(0),
        )
        .expect("weight");
    assert_eq!(weight, 5, "weights accumulate across mining runs");
}

/// The v8 `edges.rel` CHECK must admit `'co_activated'` (the memory→file
/// reinforcement edge minted by the co-activation reward). A regression in
/// the migration's CHECK list would surface here as an INSERT failure rather
/// than only inside the harder-to-read end-to-end materialize test.
#[test]
fn co_activated_rel_is_accepted_and_accumulates() {
    let conn = seed_db();
    let key = EdgeKey {
        src_kind: "memory",
        src_id: "aaaaaaa1",
        dst_kind: "file",
        dst_id: "file:r:docs/x.md",
        rel: "co_activated",
    };
    edges::insert_weighted(&conn, key, 1).expect("first co_activated insert");
    edges::insert_weighted(&conn, key, 2).expect("accumulating co_activated insert");
    let weight: i64 = conn
        .query_row(
            "SELECT weight FROM edges WHERE src_id='aaaaaaa1' \
             AND dst_id='file:r:docs/x.md' AND rel='co_activated'",
            [],
            |r| r.get(0),
        )
        .expect("co_activated weight");
    assert_eq!(weight, 3, "co_activated weights accumulate like co_changed");
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
    assert!(
        edges::outgoing(&conn, "memory", "old1", "tagged")
            .expect("outgoing")
            .is_empty()
    );
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

/// `co_changed_and_imports_edges` returns only the repo-prefixed rows, in
/// `(rel, src_id, dst_id)` order — the shape `graph::materialize::
/// project_pagerank` feeds straight into PageRank's accumulation.
#[test]
fn co_changed_and_imports_edges_filters_by_prefix_and_orders() {
    let conn = seed_db();
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "file",
            src_id: "file:r:b.rs",
            dst_kind: "file",
            dst_id: "file:r:a.rs",
            rel: "imports",
        },
    )
    .expect("insert imports r");
    edges::insert_weighted(
        &conn,
        EdgeKey {
            src_kind: "file",
            src_id: "file:r:a.rs",
            dst_kind: "file",
            dst_id: "file:r:b.rs",
            rel: "co_changed",
        },
        3,
    )
    .expect("insert co_changed r");
    // A different repo's edge must not leak in even though it shares a rel.
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "file",
            src_id: "file:other:x.rs",
            dst_kind: "file",
            dst_id: "file:other:y.rs",
            rel: "imports",
        },
    )
    .expect("insert imports other repo");

    let rows = edges::co_changed_and_imports_edges(&conn, "file:r:").expect("query");
    let shape: Vec<(&str, &str, &str, i64)> = rows
        .iter()
        .map(|r| {
            (
                r.src_id.as_str(),
                r.dst_id.as_str(),
                r.rel.as_str(),
                r.weight,
            )
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            ("file:r:a.rs", "file:r:b.rs", "co_changed", 3),
            ("file:r:b.rs", "file:r:a.rs", "imports", 1),
        ]
    );
}

/// `delete_co_changed_for_repo` removes only `co_changed` edges whose
/// `src_id` carries the given prefix — an `imports` edge and another
/// repo's `co_changed` edge both survive.
#[test]
fn delete_co_changed_for_repo_is_scoped() {
    let conn = seed_db();
    let key_co = EdgeKey {
        src_kind: "file",
        src_id: "file:r:a.rs",
        dst_kind: "file",
        dst_id: "file:r:b.rs",
        rel: "co_changed",
    };
    edges::insert_weighted(&conn, key_co, 1).expect("insert co_changed");
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "file",
            src_id: "file:r:a.rs",
            dst_kind: "file",
            dst_id: "file:r:b.rs",
            rel: "imports",
        },
    )
    .expect("insert imports");
    let other_repo = EdgeKey {
        src_kind: "file",
        src_id: "file:other:a.rs",
        dst_kind: "file",
        dst_id: "file:other:b.rs",
        rel: "co_changed",
    };
    edges::insert_weighted(&conn, other_repo, 1).expect("insert other repo co_changed");

    edges::delete_co_changed_for_repo(&conn, "file:r:").expect("delete");

    assert_eq!(edges::current_weight(&conn, key_co).expect("weight"), 0);
    assert_eq!(
        edges::outgoing(&conn, "file", "file:r:a.rs", "imports").expect("outgoing"),
        vec![("file".to_string(), "file:r:b.rs".to_string())]
    );
    assert_eq!(
        edges::current_weight(&conn, other_repo).expect("other weight"),
        1
    );
}

/// `delete_imports_from` removes only the `imports` edges sourced at the
/// node, leaving a `co_changed` edge from the same source untouched —
/// unlike `delete_outgoing`, which is rel-agnostic.
#[test]
fn delete_imports_from_is_rel_scoped() {
    let conn = seed_db();
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "file",
            src_id: "file:r:a.rs",
            dst_kind: "file",
            dst_id: "file:r:b.rs",
            rel: "imports",
        },
    )
    .expect("insert imports");
    let co_key = EdgeKey {
        src_kind: "file",
        src_id: "file:r:a.rs",
        dst_kind: "file",
        dst_id: "file:r:c.rs",
        rel: "co_changed",
    };
    edges::insert_weighted(&conn, co_key, 1).expect("insert co_changed");

    edges::delete_imports_from(&conn, "file:r:a.rs").expect("delete imports");

    assert!(
        edges::outgoing(&conn, "file", "file:r:a.rs", "imports")
            .expect("outgoing")
            .is_empty()
    );
    assert_eq!(edges::current_weight(&conn, co_key).expect("weight"), 1);
}

/// `memory_direct_relation_edges` returns memory→memory relation rows, and
/// excludes edges of an unlisted rel (e.g. `tagged`).
#[test]
fn memory_direct_relation_edges_excludes_other_rels() {
    let conn = seed_db();
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "memory",
            src_id: "new1",
            dst_kind: "memory",
            dst_id: "old1",
            rel: "supersedes",
        },
    )
    .expect("insert supersedes");
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "memory",
            src_id: "new1",
            dst_kind: "tag",
            dst_id: "tag-x",
            rel: "tagged",
        },
    )
    .expect("insert tagged");

    let rows = edges::memory_direct_relation_edges(&conn).expect("query");
    assert_eq!(rows, vec![("new1".to_string(), "old1".to_string(), 1.0)]);
}

/// `memory_co_citation_edges` weights an unordered pair by the number of
/// shared `references_file` targets.
#[test]
fn memory_co_citation_edges_counts_shared_targets() {
    let conn = seed_db();
    for (memory, file) in [
        ("m1", "a.rs"),
        ("m1", "b.rs"),
        ("m2", "a.rs"),
        ("m2", "b.rs"),
    ] {
        edges::insert(
            &conn,
            EdgeKey {
                src_kind: "memory",
                src_id: memory,
                dst_kind: "file",
                dst_id: &format!("r:{file}"),
                rel: "references_file",
            },
        )
        .expect("insert references_file");
    }

    let rows = edges::memory_co_citation_edges(&conn).expect("query");
    assert_eq!(rows, vec![("m1".to_string(), "m2".to_string(), 2.0)]);
}

/// `memory_ids_referencing_file` matches only the given `rel` and
/// `dst_id`, `src_kind='memory'`, `dst_kind='file'`.
#[test]
fn memory_ids_referencing_file_matches_rel_and_dst() {
    let conn = seed_db();
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "memory",
            src_id: "m1",
            dst_kind: "file",
            dst_id: "r:a.rs",
            rel: "references_file",
        },
    )
    .expect("insert m1");
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "memory",
            src_id: "m2",
            dst_kind: "file",
            dst_id: "r:b.rs",
            rel: "references_file",
        },
    )
    .expect("insert m2");

    let ids =
        edges::memory_ids_referencing_file(&conn, "references_file", "r:a.rs").expect("query");
    assert_eq!(ids, vec!["m1".to_string()]);
}

/// `src_ids_for_dst_ids` is the reverse batch lookup behind the
/// co-activation harvest: given a set of touched-file dst ids, it returns
/// every referencing memory, ordered `(dst_id, src_id)`.
#[test]
fn src_ids_for_dst_ids_batches_the_reverse_lookup() {
    let conn = seed_db();
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "memory",
            src_id: "m1",
            dst_kind: "file",
            dst_id: "r:a.rs",
            rel: "references_file",
        },
    )
    .expect("insert m1");
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "memory",
            src_id: "m2",
            dst_kind: "file",
            dst_id: "r:b.rs",
            rel: "references_file",
        },
    )
    .expect("insert m2");

    let rows =
        edges::src_ids_for_dst_ids(&conn, "references_file", &["r:a.rs", "r:b.rs"]).expect("query");
    assert_eq!(
        rows,
        vec![
            ("m1".to_string(), "r:a.rs".to_string()),
            ("m2".to_string(), "r:b.rs".to_string()),
        ]
    );
}

/// `file_neighbor_rows` is the raw one-hop query behind
/// `graph::neighbors::file_neighbors`: seeded from a JSON array of one
/// file id, it returns its `imports` neighbor.
#[test]
fn file_neighbor_rows_returns_the_one_hop_import_neighbor() {
    let conn = seed_db();
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "file",
            src_id: "file:r:a.rs",
            dst_kind: "file",
            dst_id: "file:r:b.rs",
            rel: "imports",
        },
    )
    .expect("insert imports");

    let seeds_json = serde_json::to_string(&vec!["file:r:a.rs"]).expect("json");
    let rows = edges::file_neighbor_rows(&conn, &seeds_json, 1).expect("query");
    assert_eq!(
        rows,
        vec![(
            "r".to_string(),
            "b.rs".to_string(),
            "imports".to_string(),
            1
        )]
    );
}
