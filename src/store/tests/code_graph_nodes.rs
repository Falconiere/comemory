#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/code_graph_nodes.rs` — the `code_symbols` node
//! aggregation behind `comemory graph`.

use crate::test_common::code_seed;
use comemory::memory::{Frontmatter, Kind, References, Relations};
use comemory::store::code_graph_nodes::{
    citing_memories, fetch_node, fetch_nodes, fetch_nodes_for_pairs, top_symbols,
};
use comemory::store::edges::{self, EdgeKey, REFERENCES_FILE, REFERENCES_SYMBOL};
use comemory::store::memory_row;
use time::OffsetDateTime;

#[test]
fn fetch_nodes_aggregates_rank_and_symbol_count_per_file() {
    let (_d, conn) = code_seed::open_db();
    code_seed::seed_symbol(&conn, "demo", "a.rs", "one");
    code_seed::seed_symbol(&conn, "demo", "a.rs", "two");
    code_seed::seed_symbol(&conn, "other", "b.rs", "three");

    let rows = fetch_nodes(&conn, Some("demo")).expect("fetch");
    assert_eq!(rows.len(), 1, "one file, two symbols, one node row");
    assert_eq!(rows[0].repo, "demo");
    assert_eq!(rows[0].path, "a.rs");
    assert_eq!(rows[0].symbols, 2);

    let all = fetch_nodes(&conn, None).expect("fetch all");
    assert_eq!(all.len(), 2, "no repo filter returns both files");
}

#[test]
fn fetch_node_is_none_for_an_unindexed_file() {
    let (_d, conn) = code_seed::open_db();
    assert!(
        fetch_node(&conn, "demo", "missing.rs")
            .expect("fetch")
            .is_none()
    );
}

#[test]
fn fetch_node_returns_the_one_row_for_an_indexed_file() {
    let (_d, conn) = code_seed::open_db();
    code_seed::seed_symbol(&conn, "demo", "a.rs", "one");
    let row = fetch_node(&conn, "demo", "a.rs")
        .expect("fetch")
        .expect("row present");
    assert_eq!(row.repo, "demo");
    assert_eq!(row.symbols, 1);
}

#[test]
fn fetch_nodes_for_pairs_skips_pairs_with_no_code_symbols_rows() {
    let (_d, conn) = code_seed::open_db();
    code_seed::seed_symbol(&conn, "demo", "a.rs", "one");
    let pairs = vec![
        ("demo".to_string(), "a.rs".to_string()),
        ("demo".to_string(), "never-indexed.rs".to_string()),
    ];
    let rows = fetch_nodes_for_pairs(&conn, &pairs).expect("fetch");
    assert_eq!(rows.len(), 1, "the stale endpoint produces no row");
    assert_eq!(rows[0].path, "a.rs");
}

#[test]
fn top_symbols_orders_by_rank_score_then_symbol_then_line() {
    let (_d, conn) = code_seed::open_db();
    code_seed::seed_symbol(&conn, "demo", "a.rs", "low");
    code_seed::seed_symbol(&conn, "demo", "a.rs", "high");
    conn.execute(
        "UPDATE code_symbols SET rank_score = 0.1 WHERE symbol = 'low'",
        [],
    )
    .expect("set low rank");
    conn.execute(
        "UPDATE code_symbols SET rank_score = 0.9 WHERE symbol = 'high'",
        [],
    )
    .expect("set high rank");

    let rows = top_symbols(&conn, "demo", "a.rs", 10).expect("top_symbols");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].symbol, "high", "strongest rank_score first");
    assert_eq!(rows[1].symbol, "low");
}

#[test]
fn top_symbols_respects_the_limit_and_excludes_chunk_children() {
    let (_d, conn) = code_seed::open_db();
    let parent_id = code_seed::seed_symbol(&conn, "demo", "a.rs", "parent");
    code_seed::seed_symbol(&conn, "demo", "a.rs", "sibling");
    conn.execute(
        "INSERT INTO code_symbols(\
             repo, path, blob_oid, symbol, kind, lang, line_start, line_end, \
             snippet, simhash, parent_id, indexed_at) \
         VALUES('demo', 'a.rs', 'oid', 'parent#1', 'function', 'rust', 1, 5, 'x', 0, ?1, \
                strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        rusqlite::params![parent_id],
    )
    .expect("seed chunk child");

    let rows = top_symbols(&conn, "demo", "a.rs", 1).expect("top_symbols");
    assert_eq!(rows.len(), 1, "limit=1 caps the page");
    let capped = top_symbols(&conn, "demo", "a.rs", 10).expect("uncapped");
    assert!(
        capped.iter().all(|r| r.symbol != "parent#1"),
        "chunk children are excluded: {:?}",
        capped.iter().map(|r| &r.symbol).collect::<Vec<_>>()
    );
    assert_eq!(capped.len(), 2, "parent + sibling, not the chunk child");
}

/// Insert one memory row with an explicit id via the production writer.
fn seed_memory(conn: &rusqlite::Connection, id: &str, body: &str) {
    let fm = Frontmatter {
        id: id.to_string(),
        kind: Kind::Note,
        repo: "demo".to_string(),
        tags: Vec::new(),
        author: "alice".to_string(),
        created: OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("valid ts"),
        quality: 3,
        schema: 1,
        content_hash: format!("hash-{id}"),
        references: References::default(),
        relations: Relations::default(),
    };
    memory_row::insert(
        conn,
        &fm,
        body,
        "note",
        &format!("/data/.comemory/memories/{id}-note.md"),
        &[],
    )
    .expect("insert memory");
}

#[test]
fn citing_memories_matches_the_cites_file_predicate_and_stays_distinct() {
    let (_d, conn) = code_seed::open_db();
    seed_memory(&conn, "aaaaaaaa", "cites the file twice");
    seed_memory(&conn, "bbbbbbbb", "cites nothing");
    // A references_file edge AND a references_symbol edge from the SAME
    // memory into the SAME file must collapse to one row (DISTINCT),
    // matching `cites_file_predicate`'s `COUNT(DISTINCT src_id)` semantics
    // for the node's `memories` count.
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "memory",
            src_id: "aaaaaaaa",
            dst_kind: "file",
            dst_id: "demo:a.rs",
            rel: REFERENCES_FILE,
        },
    )
    .expect("insert file edge");
    edges::insert(
        &conn,
        EdgeKey {
            src_kind: "memory",
            src_id: "aaaaaaaa",
            dst_kind: "symbol",
            dst_id: "demo:a.rs:alpha",
            rel: REFERENCES_SYMBOL,
        },
    )
    .expect("insert symbol edge");

    let rows = citing_memories(&conn, "demo", "a.rs").expect("citing_memories");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "aaaaaaaa");
    assert_eq!(rows[0].body, "cites the file twice");

    let untouched = citing_memories(&conn, "demo", "b.rs").expect("citing_memories b");
    assert!(untouched.is_empty());
}
