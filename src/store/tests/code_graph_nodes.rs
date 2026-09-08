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
use comemory::store::code_graph_nodes::{fetch_node, fetch_nodes, fetch_nodes_for_pairs};

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
