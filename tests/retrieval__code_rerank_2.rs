//! Tests for [`comemory::retrieval::code_rerank`] (part 2): chunk coalescing
//! and score-parts invariants.

#[path = "common/code_rerank_support.rs"]
mod support;

use comemory::config::Config;
use comemory::retrieval::code_rerank::{WorkingSet, rerank_code};
use comemory::retrieval::router::Source;

#[test]
fn chunks_coalesce_onto_parent_identity() {
    let (_d, conn) = support::open_db();
    let cfg = Config::defaults();
    let parent = support::seed(&conn, "demo", "big.rs", "big_fn", (1, 100), None);
    let c0 = support::seed(&conn, "demo", "big.rs", "big_fn#0", (1, 50), Some(parent));
    let c1 = support::seed(&conn, "demo", "big.rs", "big_fn#1", (51, 100), Some(parent));

    let out = rerank_code(
        &conn,
        &cfg,
        &[support::hit(c0, 1.0), support::hit(c1, 0.5)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out.len(), 1, "two chunks of one parent → one output row");
    let row = &out[0];
    assert_eq!(row.symbol_id, parent, "output carries the parent id");
    assert_eq!(row.symbol, "big_fn", "output carries the parent symbol");
    assert_eq!(row.kind, "function");
    assert_eq!(
        (row.line_start, row.line_end),
        (1, 50),
        "output keeps the winning chunk's line range"
    );
    assert!(
        (f64::from(row.parts.relevance) - 1.0).abs() < 1e-6,
        "output keeps the best chunk's score"
    );
}

#[test]
fn parent_and_chunks_in_one_pool_coalesce_to_single_row() {
    let (_d, conn) = support::open_db();
    let cfg = Config::defaults();
    let parent = support::seed(&conn, "demo", "big.rs", "big_fn", (1, 100), None);
    let c0 = support::seed(&conn, "demo", "big.rs", "big_fn#0", (1, 50), Some(parent));
    let c1 = support::seed(&conn, "demo", "big.rs", "big_fn#1", (51, 100), Some(parent));

    // Chunk c1 carries the highest route score: the group must collapse
    // to one row with the parent's identity but c1's line range.
    let out = rerank_code(
        &conn,
        &cfg,
        &[
            support::hit(parent, 0.6),
            support::hit(c0, 0.4),
            support::hit(c1, 1.0),
        ],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out.len(), 1, "parent + two chunks → one output row");
    assert_eq!(out[0].symbol_id, parent, "output carries the parent id");
    assert_eq!(out[0].symbol, "big_fn", "output carries the parent symbol");
    assert_eq!(
        (out[0].line_start, out[0].line_end),
        (51, 100),
        "output keeps the winning chunk's line range"
    );

    // Parent-wins variant: bump the parent's route score highest and the
    // single output row must keep the parent's OWN line range — no
    // identity swap onto a chunk's narrower span.
    let out = rerank_code(
        &conn,
        &cfg,
        &[
            support::hit(parent, 1.0),
            support::hit(c0, 0.5),
            support::hit(c1, 0.4),
        ],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out.len(), 1, "parent + two chunks → one output row");
    assert_eq!(out[0].symbol_id, parent, "output carries the parent id");
    assert_eq!(out[0].symbol, "big_fn", "output carries the parent symbol");
    assert_eq!(
        (out[0].line_start, out[0].line_end),
        (1, 100),
        "winning parent keeps its own line range"
    );
}

#[test]
fn score_parts_product_invariant() {
    let (_d, conn) = support::open_db();
    let cfg = Config::defaults();
    let a = support::seed(&conn, "demo", "a.rs", "a::run", (1, 10), None);
    let b = support::seed(&conn, "demo", "b.rs", "b::run", (1, 10), None);
    // Make every prior non-trivial for `a`.
    conn.execute(
        "UPDATE code_symbols SET rank_score = 0.7, access_count = 9 WHERE id = ?1",
        [a],
    )
    .expect("bump signals");
    conn.execute(
        "UPDATE code_symbols SET rank_score = 0.3 WHERE id = ?1",
        [b],
    )
    .expect("set rank");
    conn.execute(
        "INSERT INTO code_feedback(repo, path, symbol, used_count, irrelevant_count) \
         VALUES ('demo', 'a.rs', 'a::run', 6, 1)",
        [],
    )
    .expect("seed feedback");

    let out = rerank_code(
        &conn,
        &cfg,
        &[support::hit(a, 8.0), support::hit(b, 2.0)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out.len(), 2);
    for r in &out {
        let product = f64::from(r.parts.relevance)
            * r.parts.rank
            * r.parts.activation
            * r.parts.affinity
            * r.parts.feedback;
        assert!(
            (r.parts.final_score - product).abs() < 1e-6,
            "invariant broken for {}: final {} vs product {}",
            r.symbol_id,
            r.parts.final_score,
            product
        );
        // Empty working set → affinity exactly neutral.
        assert!((r.parts.affinity - 1.0).abs() < 1e-12);
    }
}

#[test]
fn vanished_rows_are_dropped() {
    let (_d, conn) = support::open_db();
    let cfg = Config::defaults();
    let a = support::seed(&conn, "demo", "a.rs", "a::run", (1, 10), None);
    let out = rerank_code(
        &conn,
        &cfg,
        &[support::hit(a, 1.0), support::hit(9_999, 1.0)],
        &WorkingSet::default(),
    )
    .expect("rerank");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].symbol_id, a);
    assert_eq!(out[0].source, Source::Lexical);
    assert_eq!(
        (out[0].repo.as_str(), out[0].path.as_str()),
        ("demo", "a.rs")
    );
    assert_eq!(out[0].lang, "rust");
}
