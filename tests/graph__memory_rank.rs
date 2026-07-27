//! Behavior tests for [`comemory::graph::memory_rank`] — PageRank over the
//! derived memory graph, materialized onto `memories.rank_score`.
//!
//! Every test runs against a real migrated `comemory.db`. Rows are seeded
//! by raw INSERT in the exact forms the production writers emit: bare 8-hex
//! memory ids on both sides of a relation edge (`memory_row::
//! insert_relation_edges`), bare `<repo>:<path>[:<symbol>]` reference
//! targets (`cross_link::extract_and_emit`), and `file:`-prefixed
//! `co_activated` targets (`coactivate::harvest`).

use std::collections::BTreeMap;

use comemory::graph::memory_rank;
use rusqlite::{Connection, params};

/// A freshly migrated `comemory.db` inside a tempdir.
fn open_db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// Insert one memory row with `rank_score` left at the 0.0 column default,
/// so whatever a test reads back was written by a materialize run. A
/// `deleted_at` makes the row soft-deleted, i.e. outside the node universe.
fn seed_memory(conn: &Connection, id: &str, deleted_at: Option<&str>) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, content_hash, body,
                              created_at, updated_at, deleted_at, md_path)
         VALUES(?1, 'note', 'decision', 'demo', 'f', 'h', 'body',
                '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z', ?2, 'm/x.md')",
        params![id, deleted_at],
    )
    .expect("seed memory");
}

/// Insert one outgoing edge from memory `src`, weight left at the default 1.
fn seed_edge(conn: &Connection, src: &str, rel: &str, dst_kind: &str, dst: &str) {
    conn.execute(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES('memory', ?1, ?2, ?3, ?4, '2026-07-01T00:00:00Z')",
        params![src, dst_kind, dst, rel],
    )
    .expect("seed edge");
}

/// Memory→memory relation edge: bare 8-hex ids on both sides.
fn seed_relation(conn: &Connection, src: &str, rel: &str, dst: &str) {
    seed_edge(conn, src, rel, "memory", dst);
}

/// Memory→symbol reference edge: BARE `<repo>:<path>:<symbol>` dst, with no
/// `symbol:` kind prefix — the form `cross_link` actually writes.
fn seed_symbol_ref(conn: &Connection, src: &str, target: &str) {
    seed_edge(conn, src, "references_symbol", "symbol", target);
}

/// Every `memories.rank_score`, keyed by id — soft-deleted rows included,
/// so a test can assert they were left untouched.
fn read_ranks(conn: &Connection) -> BTreeMap<String, f64> {
    conn.prepare("SELECT id, rank_score FROM memories ORDER BY id")
        .expect("prepare")
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
}

/// Recompute, then read every score back.
fn materialized_ranks(conn: &mut Connection) -> BTreeMap<String, f64> {
    memory_rank::materialize_memory_rank(conn).expect("materialize");
    read_ranks(conn)
}

/// Score for `id`, panicking with the id when it is absent.
fn score(ranks: &BTreeMap<String, f64>, id: &str) -> f64 {
    *ranks
        .get(id)
        .unwrap_or_else(|| panic!("no rank_score row for {id}"))
}

#[test]
fn derived_from_hub_outranks_isolated_peer_and_its_referrers() {
    let (_dir, mut conn) = open_db();
    let referrers = ["aaaa0001", "aaaa0002", "aaaa0003"];
    for id in referrers.iter().chain(&["bbbb0001", "dddd0001"]) {
        seed_memory(&conn, id, None);
    }
    // Three memories build on the hub; dddd0001 is linked to nothing.
    for src in referrers {
        seed_relation(&conn, src, "derived_from", "bbbb0001");
    }

    let ranks = materialized_ranks(&mut conn);
    let hub = score(&ranks, "bbbb0001");
    let isolated = score(&ranks, "dddd0001");

    assert!(hub > isolated, "hub {hub} !> isolated peer {isolated}");
    for src in referrers {
        let referrer = score(&ranks, src);
        assert!(referrer < hub, "referrer {src} {referrer} !< hub {hub}");
        // Mass flows TO the built-upon memory: emitting an edge never
        // enriches its own source, so a referrer stays at the baseline.
        assert!(
            (referrer - isolated).abs() < 1e-12,
            "referrer {src} {referrer} left the teleport baseline {isolated}"
        );
    }
}

#[test]
fn co_citation_lifts_both_memories_symmetrically() {
    let (_dir, mut conn) = open_db();
    for id in ["aaaa0001", "aaaa0002", "cccc0001"] {
        seed_memory(&conn, id, None);
    }
    // Both cite the same symbol; cccc0001 cites nothing.
    seed_symbol_ref(&conn, "aaaa0001", "demo:src/store/mod.rs:open");
    seed_symbol_ref(&conn, "aaaa0002", "demo:src/store/mod.rs:open");

    let ranks = materialized_ranks(&mut conn);
    let (m1, m2) = (score(&ranks, "aaaa0001"), score(&ranks, "aaaa0002"));
    let isolated = score(&ranks, "cccc0001");

    // The derived edge is undirected — neither co-citer is the donor.
    assert!(
        (m1 - m2).abs() < 1e-12,
        "asymmetric co-citation: {m1} vs {m2}"
    );
    assert!(m1 > isolated, "co-citer {m1} !> isolated {isolated}");
    assert!(m2 > isolated, "co-citer {m2} !> isolated {isolated}");
}

#[test]
fn hub_relations_only_corpus_ranks_uniformly() {
    let (_dir, mut conn) = open_db();
    let ids = ["aaaa0001", "aaaa0002", "aaaa0003"];
    for id in ids {
        seed_memory(&conn, id, None);
    }
    // Shared repo, author and tag — the rels that would connect the whole
    // corpus through hub nodes, and are deliberately excluded.
    for id in ids {
        seed_edge(&conn, id, "in_repo", "repo", "repo:demo");
        seed_edge(&conn, id, "authored_by", "author", "author:f");
        seed_edge(&conn, id, "tagged", "tag", "tag:sqlite");
    }

    let (nodes, edges) = memory_rank::derive_memory_graph(&conn).expect("derive");
    assert_eq!(nodes.len(), 3);
    assert!(
        edges.is_empty(),
        "hub rels must not enter the graph: {edges:?}"
    );

    let ranks = materialized_ranks(&mut conn);
    let baseline = score(&ranks, ids[0]);
    assert!(baseline > 0.0, "isolated nodes keep the teleport baseline");
    for id in ids {
        let got = score(&ranks, id);
        assert!(
            (got - baseline).abs() < 1e-12,
            "{id} at {got} diverged from the uniform {baseline}"
        );
    }
}

/// Live ids of the mixed corpus used by the determinism tests.
const LINKED_IDS: [&str; 4] = ["aaaa0001", "aaaa0002", "aaaa0003", "bbbb0001"];

/// The mixed corpus's logical edges as `(src, rel, dst_kind, dst)`,
/// exercising both edge sources and all three co-citation rel kinds. Kept
/// as data so a test can permute the insert order and replay the same
/// logical graph.
fn linked_corpus_edges() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    vec![
        ("aaaa0001", "derived_from", "memory", "bbbb0001"),
        ("aaaa0002", "derived_from", "memory", "bbbb0001"),
        ("aaaa0003", "supersedes", "memory", "aaaa0001"),
        (
            "aaaa0001",
            "references_symbol",
            "symbol",
            "demo:src/a.rs:open",
        ),
        (
            "aaaa0002",
            "references_symbol",
            "symbol",
            "demo:src/a.rs:open",
        ),
        ("aaaa0003", "references_file", "file", "demo:src/b.rs"),
        ("bbbb0001", "references_file", "file", "demo:src/b.rs"),
        ("aaaa0001", "co_activated", "file", "file:demo:src/c.rs"),
        ("bbbb0001", "co_activated", "file", "file:demo:src/c.rs"),
    ]
}

/// Seed [`LINKED_IDS`] plus `edges`, in the order given.
fn seed_linked_corpus(conn: &Connection, edges: &[(&str, &str, &str, &str)]) {
    for id in LINKED_IDS {
        seed_memory(conn, id, None);
    }
    for (src, rel, dst_kind, dst) in edges {
        seed_edge(conn, src, rel, dst_kind, dst);
    }
}

#[test]
fn recomputing_is_bit_identical() {
    let (_dir, mut conn) = open_db();
    seed_linked_corpus(&conn, &linked_corpus_edges());

    let first = materialized_ranks(&mut conn);
    let second = materialized_ranks(&mut conn);

    assert_eq!(first, second, "a second recompute drifted");
}

#[test]
fn edge_insert_order_does_not_change_scores() {
    let mut edges = linked_corpus_edges();
    let (_dir_a, mut forward_db) = open_db();
    seed_linked_corpus(&forward_db, &edges);
    let forward = materialized_ranks(&mut forward_db);

    edges.reverse();
    let (_dir_b, mut reversed_db) = open_db();
    seed_linked_corpus(&reversed_db, &edges);
    let reversed = materialized_ranks(&mut reversed_db);

    // Without the ORDER BYs, rowid order would perturb the f64
    // accumulation and the two runs would differ in the last ulps.
    assert_eq!(forward, reversed, "rowid order leaked into the scores");
}

#[test]
fn derive_keeps_relations_directed_and_expands_co_citation_both_ways() {
    let (_dir, conn) = open_db();
    seed_memory(&conn, "aaaa0001", None);
    seed_memory(&conn, "bbbb0002", None);
    seed_relation(&conn, "aaaa0001", "derived_from", "bbbb0002");
    seed_symbol_ref(&conn, "aaaa0001", "demo:src/a.rs:open");
    seed_symbol_ref(&conn, "bbbb0002", "demo:src/a.rs:open");

    let (nodes, edges) = memory_rank::derive_memory_graph(&conn).expect("derive");

    // Node ids are sorted, so the dense index is 0 = aaaa0001, 1 = bbbb0002.
    assert_eq!(nodes, vec!["aaaa0001".to_string(), "bbbb0002".to_string()]);
    // Direct relations first (directed, as stored), then the co-citation
    // edge expanded to both directions.
    assert_eq!(edges, vec![(0, 1, 1.0), (0, 1, 1.0), (1, 0, 1.0)]);
}

#[test]
fn dangling_relation_target_is_tolerated() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa0001", None);
    seed_memory(&conn, "aaaa0002", None);
    // Hand-edited frontmatter may name an id that was never saved.
    seed_relation(&conn, "aaaa0001", "derived_from", "deadbeef");

    let (nodes, edges) = memory_rank::derive_memory_graph(&conn).expect("derive");
    assert_eq!(nodes, vec!["aaaa0001".to_string(), "aaaa0002".to_string()]);
    assert!(
        edges.is_empty(),
        "dangling target must be skipped: {edges:?}"
    );

    let ranks = materialized_ranks(&mut conn);
    assert!(score(&ranks, "aaaa0001") > 0.0);
}

#[test]
fn empty_corpus_is_a_no_op() {
    let (_dir, mut conn) = open_db();

    let (nodes, edges) = memory_rank::derive_memory_graph(&conn).expect("derive");
    assert!(nodes.is_empty());
    assert!(edges.is_empty());

    memory_rank::materialize_memory_rank(&mut conn).expect("no-op on an empty corpus");
    assert!(read_ranks(&conn).is_empty());
}

#[test]
fn soft_deleted_memory_leaves_the_node_universe() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa0001", None);
    seed_memory(&conn, "aaaa0002", None);
    seed_memory(&conn, "cccc0003", Some("2026-07-02T00:00:00Z"));
    seed_relation(&conn, "cccc0003", "derived_from", "aaaa0001");

    let (nodes, edges) = memory_rank::derive_memory_graph(&conn).expect("derive");
    assert_eq!(nodes, vec!["aaaa0001".to_string(), "aaaa0002".to_string()]);
    assert!(
        edges.is_empty(),
        "a deleted endpoint drops its edge: {edges:?}"
    );

    let ranks = materialized_ranks(&mut conn);
    assert!(score(&ranks, "aaaa0001") > 0.0);
    // Never written, so the deleted row keeps the 0.0 column default.
    assert_eq!(score(&ranks, "cccc0003"), 0.0);
}

#[test]
fn refresh_best_effort_matches_the_fallible_form() {
    let edges = linked_corpus_edges();
    let (_dir_a, mut fallible) = open_db();
    seed_linked_corpus(&fallible, &edges);
    let expected = materialized_ranks(&mut fallible);

    let (_dir_b, mut best_effort) = open_db();
    seed_linked_corpus(&best_effort, &edges);
    memory_rank::refresh_best_effort(&mut best_effort);

    assert_eq!(read_ranks(&best_effort), expected);
}
