//! Behavior tests for [`comemory::graph::derived`] — the single post-write
//! pass that refreshes `memories.rank_score` and the `edge_fts` triplet
//! index together.
//!
//! Every test runs against a real migrated `comemory.db`, with edges seeded
//! by raw INSERT in the exact forms the production writers emit: bare 8-hex
//! memory ids on both sides of a relation edge, and `file:`-prefixed
//! `co_activated` targets (`coactivate::harvest`, which `index-code` drives
//! through `graph::materialize`).

use std::collections::BTreeMap;

use comemory::graph::derived;
use rusqlite::{Connection, params};

/// A freshly migrated `comemory.db` inside a tempdir.
fn open_db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// Insert one live memory row, leaving `rank_score` at the 0.0 column
/// default so whatever a test reads back was written by a refresh.
fn seed_memory(conn: &Connection, id: &str, kind: &str, slug: &str) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, content_hash, body,
                              created_at, updated_at, md_path)
         VALUES(?1, ?2, ?3, 'demo', 'f', 'h', 'body',
                '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z', 'm/x.md')",
        params![id, slug, kind],
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

/// Every `memories.rank_score`, keyed by id.
fn read_ranks(conn: &Connection) -> BTreeMap<String, f64> {
    let mut stmt = conn
        .prepare("SELECT id, rank_score FROM memories ORDER BY id")
        .expect("prepare ranks");
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .expect("rank query")
        .collect::<Result<BTreeMap<_, _>, _>>()
        .expect("rank rows")
}

/// Number of indexed triplets.
fn edge_fts_rows(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM edge_fts", [], |r| r.get(0))
        .expect("edge_fts count")
}

#[test]
fn wrapper_refreshes_both_derived_artifacts_in_one_pass() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_memory(&conn, "bbbb2222", "note", "queue-design-v1");
    seed_edge(&conn, "aaaa1111", "supersedes", "memory", "bbbb2222");

    // Both artifacts start at their untouched defaults.
    assert_eq!(read_ranks(&conn).values().copied().sum::<f64>(), 0.0);
    assert_eq!(edge_fts_rows(&conn), 0);

    derived::refresh_derived_best_effort(&mut conn);

    let ranks = read_ranks(&conn);
    assert!(
        ranks.values().all(|s| *s > 0.0),
        "rank_score not materialized: {ranks:?}"
    );
    // PageRank mass flows along the edge, so the superseded memory outranks
    // the one pointing at it — proof the graph, not a flat seed, was scored.
    assert!(ranks["bbbb2222"] > ranks["aaaa1111"]);
    assert_eq!(edge_fts_rows(&conn), 1);
}

#[test]
fn each_artifact_is_independently_best_effort() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_memory(&conn, "bbbb2222", "note", "queue-design-v1");
    seed_edge(&conn, "aaaa1111", "supersedes", "memory", "bbbb2222");

    // Break the second artifact only. The wrapper must still complete the
    // first and return normally rather than propagating or panicking.
    conn.execute("DROP TABLE edge_fts", [])
        .expect("drop edge_fts");

    derived::refresh_derived_best_effort(&mut conn);

    let ranks = read_ranks(&conn);
    assert!(
        ranks.values().all(|s| *s > 0.0),
        "a failing edge_fts refresh must not cost rank freshness: {ranks:?}"
    );
}

#[test]
fn refresh_is_idempotent_across_repeated_passes() {
    let (_dir, mut conn) = open_db();
    seed_memory(&conn, "aaaa1111", "decision", "queue-design-v2");
    seed_memory(&conn, "bbbb2222", "note", "queue-design-v1");
    seed_edge(&conn, "aaaa1111", "supersedes", "memory", "bbbb2222");

    derived::refresh_derived_best_effort(&mut conn);
    let first = read_ranks(&conn);
    derived::refresh_derived_best_effort(&mut conn);

    // A second pass over an unchanged graph changes nothing — the seams
    // fire on every save, so drift here would be visible immediately.
    assert_eq!(read_ranks(&conn), first);
    assert_eq!(edge_fts_rows(&conn), 1);
}

/// [AC-5] The `index-code` seam: `graph::materialize` writes `co_activated`
/// edges (commits touching a memory's referenced files reinforce it), and
/// before F4 nothing refreshed `memories.rank_score` afterwards. Exercised
/// at the wrapper level — the seam `cli::index_code::graph_post_pass` now
/// calls; the real-binary path is covered by the `index-code` e2e.
#[test]
fn co_activated_edges_written_by_index_code_reach_rank_score() {
    let (_dir, mut conn) = open_db();
    for id in ["aaaa1111", "bbbb2222", "cccc3333"] {
        seed_memory(&conn, id, "decision", "queue-design");
    }
    // A relation edge alone gives every memory a baseline score.
    seed_edge(&conn, "aaaa1111", "supersedes", "memory", "bbbb2222");
    derived::refresh_derived_best_effort(&mut conn);
    let before = read_ranks(&conn);

    // Now the `co_activated` co-citation `materialize` would have written:
    // two memories citing the same file, `file:`-prefixed as `coactivate`
    // emits them.
    seed_edge(
        &conn,
        "bbbb2222",
        "co_activated",
        "file",
        "file:demo:src/queue.rs",
    );
    seed_edge(
        &conn,
        "cccc3333",
        "co_activated",
        "file",
        "file:demo:src/queue.rs",
    );

    derived::refresh_derived_best_effort(&mut conn);

    let after = read_ranks(&conn);
    assert_ne!(
        after["cccc3333"], before["cccc3333"],
        "co_activated co-citation did not reach rank_score"
    );
    assert!(after["cccc3333"] > before["cccc3333"]);
    // And the same pass indexed all three edges as triplets.
    assert_eq!(edge_fts_rows(&conn), 3);
}
