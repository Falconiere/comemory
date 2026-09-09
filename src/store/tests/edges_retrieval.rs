#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/edges_retrieval.rs` — the memory
//! graph-expansion walk, the context relation walk, the co-change affinity
//! sum, and the live-supersede lookup. Real `memories`/`edges` rows via the
//! production writers, no mocks. Order is asserted explicitly wherever the
//! production query has an `ORDER BY`.

use comemory::memory::{Frontmatter, Kind, References, Relations};
use comemory::store::edges::{self, EdgeKey};
use comemory::store::edges_retrieval::{
    SeedWalk, co_change_weight, expand_memory_seeds, live_superseder, walk_context_edges,
};
use comemory::store::{connection, memory_row};
use rusqlite::Connection;
use time::OffsetDateTime;

fn seed_db() -> Connection {
    let dir = tempfile::tempdir().expect("tempdir");
    connection::open(dir.path().join("comemory.db")).expect("open")
}

/// Insert one memory row with an explicit id and `created` offset (whole
/// days from a fixed epoch; larger = more recent) via the production writer.
fn seed_memory(conn: &Connection, id: &str, day: i64) {
    let created =
        OffsetDateTime::from_unix_timestamp(1_700_000_000 + day * 86_400).expect("valid ts");
    let fm = Frontmatter {
        id: id.to_string(),
        kind: Kind::Note,
        repo: "r".to_string(),
        tags: Vec::new(),
        author: "alice".to_string(),
        created,
        quality: 3,
        schema: 1,
        content_hash: format!("hash-{id}"),
        references: References::default(),
        relations: Relations::default(),
    };
    memory_row::insert(
        conn,
        &fm,
        "body",
        "note",
        &format!("/data/.comemory/memories/{id}-note.md"),
        &[],
    )
    .expect("insert memory");
}

fn edge(conn: &Connection, src: &str, dst: &str, rel: &str) {
    edges::insert(
        conn,
        EdgeKey {
            src_kind: "memory",
            src_id: src,
            dst_kind: "memory",
            dst_id: dst,
            rel,
        },
    )
    .expect("insert edge");
}

#[test]
fn expand_memory_seeds_orders_by_hops_then_id_and_excludes_seeds() {
    let conn = seed_db();
    for id in ["aaaa0001", "bbbb0002", "cccc0003", "dddd0004"] {
        seed_memory(&conn, id, 0);
    }
    // aaaa0001 --relates_to--> bbbb0002 (1 hop)
    // aaaa0001 --relates_to--> cccc0003 via bbbb0002 is NOT how CTE reaches it;
    // instead give cccc0003 a direct 1-hop edge too, and dddd0004 a 2-hop path
    // through bbbb0002, so ordering must place the two 1-hop ids first.
    edge(&conn, "aaaa0001", "bbbb0002", "relates_to");
    edge(&conn, "aaaa0001", "cccc0003", "relates_to");
    edge(&conn, "bbbb0002", "dddd0004", "relates_to");

    let seeds = serde_json::to_string(&["aaaa0001"]).expect("json");
    let rels = "'relates_to'";
    let rows = expand_memory_seeds(
        &conn,
        &SeedWalk {
            rels_clause: rels,
            seeds_json: &seeds,
            hops: 4,
            max_walk: 4096,
            repo: None,
            kind: None,
            since: None,
            cutoff: None,
            cap: 256,
        },
    )
    .expect("walk");

    // Assert the seed's absence directly rather than inferring it from the
    // expected vector: a regression that leaked the seed back in would fail
    // the vector comparison with a confusing diff, where this names the rule.
    assert!(
        !rows.iter().any(|(id, _)| id == "aaaa0001"),
        "the seed must never appear in its own expansion"
    );
    assert_eq!(
        rows,
        vec![
            ("bbbb0002".to_string(), 1),
            ("cccc0003".to_string(), 1),
            ("dddd0004".to_string(), 2),
        ],
        "must order (hops ASC, id ASC) and never include the seed itself"
    );
}

#[test]
fn expand_memory_seeds_empty_seeds_json_is_empty() {
    let conn = seed_db();
    seed_memory(&conn, "aaaa0001", 0);
    let seeds = serde_json::to_string(&Vec::<String>::new()).expect("json");
    let rows = expand_memory_seeds(
        &conn,
        &SeedWalk {
            rels_clause: "'relates_to'",
            seeds_json: &seeds,
            hops: 2,
            max_walk: 4096,
            repo: None,
            kind: None,
            since: None,
            cutoff: None,
            cap: 256,
        },
    )
    .expect("walk");
    assert!(rows.is_empty());
}

#[test]
fn walk_context_edges_orders_by_rel_then_node_and_walks_two_hops() {
    let conn = seed_db();
    for id in ["aaaa0001", "bbbb0002", "cccc0003"] {
        seed_memory(&conn, id, 0);
    }
    edge(&conn, "aaaa0001", "bbbb0002", "supersedes");
    edge(&conn, "bbbb0002", "cccc0003", "relates_to");

    let rows = walk_context_edges(&conn, "aaaa0001", 2).expect("walk");
    let shaped: Vec<(String, String, String)> = rows
        .iter()
        .map(|e| (e.rel.clone(), e.src_id.clone(), e.dst_id.clone()))
        .collect();
    assert_eq!(
        shaped,
        vec![
            (
                "relates_to".to_string(),
                "bbbb0002".to_string(),
                "cccc0003".to_string()
            ),
            (
                "supersedes".to_string(),
                "aaaa0001".to_string(),
                "bbbb0002".to_string()
            ),
        ],
        "must order by (rel, src_kind, src_id, dst_kind, dst_id)"
    );
}

#[test]
fn walk_context_edges_depth_zero_neighbor_finds_nothing_beyond_max_depth() {
    let conn = seed_db();
    for id in ["aaaa0001", "bbbb0002", "cccc0003"] {
        seed_memory(&conn, id, 0);
    }
    edge(&conn, "aaaa0001", "bbbb0002", "supersedes");
    edge(&conn, "bbbb0002", "cccc0003", "relates_to");

    let rows = walk_context_edges(&conn, "aaaa0001", 1).expect("walk");
    assert_eq!(rows.len(), 1, "depth 1 must not reach the second hop");
    assert_eq!(rows[0].dst_id, "bbbb0002");
}

#[test]
fn co_change_weight_sums_both_orientations() {
    let conn = seed_db();
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
    .expect("insert weighted");
    edges::insert_weighted(
        &conn,
        EdgeKey {
            src_kind: "file",
            src_id: "file:r:c.rs",
            dst_kind: "file",
            dst_id: "file:r:a.rs",
            rel: "co_changed",
        },
        5,
    )
    .expect("insert weighted");

    let ws = vec!["file:r:b.rs".to_string(), "file:r:c.rs".to_string()];
    let w = co_change_weight(&conn, "file:r:a.rs", &ws).expect("query");
    assert!(
        (w - 8.0).abs() < 1e-12,
        "must sum both orientations, got {w}"
    );
}

#[test]
fn co_change_weight_with_no_edges_is_zero() {
    let conn = seed_db();
    let w = co_change_weight(&conn, "file:r:a.rs", &["file:r:b.rs".to_string()]).expect("query");
    assert_eq!(w, 0.0);
}

#[test]
fn live_superseder_picks_earliest_live_superseder_and_respects_as_of_cutoff() {
    let conn = seed_db();
    seed_memory(&conn, "aaaa0001", 0); // superseded
    seed_memory(&conn, "bbbb0002", 1); // earlier superseder
    seed_memory(&conn, "cccc0003", 2); // later superseder
    edge(&conn, "bbbb0002", "aaaa0001", "supersedes");
    edge(&conn, "cccc0003", "aaaa0001", "supersedes");

    let found = live_superseder(&conn, "aaaa0001", None).expect("query");
    assert_eq!(
        found,
        Some("bbbb0002".to_string()),
        "earliest superseder wins"
    );

    // An `as_of_cutoff` before bbbb0002's own `created_at` excludes it too.
    let cutoff = memory_row::iso_format(
        OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("valid ts"),
    )
    .expect("format");
    let found_at_cutoff = live_superseder(&conn, "aaaa0001", Some(&cutoff)).expect("query");
    assert_eq!(
        found_at_cutoff, None,
        "a cutoff before every superseder's created_at excludes them all"
    );
}

#[test]
fn live_superseder_none_when_unsuperseded() {
    let conn = seed_db();
    seed_memory(&conn, "aaaa0001", 0);
    assert_eq!(
        live_superseder(&conn, "aaaa0001", None).expect("query"),
        None
    );
}
