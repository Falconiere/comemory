//! Tests for [`comemory::retrieval::code_prior`] — the four-prior product
//! shared by `code_rerank` (relevance × priors, covered in
//! `tests/retrieval/code_rerank.rs`) and `bundle` (priors only, covered in
//! `tests/retrieval/bundle.rs`). These tests pin the standalone helpers:
//! [`prior_product`] and [`pool_median_rank`] against real seeded rows.

use comemory::config::Config;
use comemory::retrieval::code_prior::{pool_median_rank, prior_product};
use comemory::retrieval::code_rerank::WorkingSet;

use super::code_seed;

#[test]
fn prior_product_is_none_for_vanished_symbol() {
    let (_d, conn) = code_seed::open_db();
    let cfg = Config::defaults();
    let parts =
        prior_product(&conn, &cfg, 9_999, &WorkingSet::default(), 0.0).expect("prior_product");
    assert!(parts.is_none(), "missing code_symbols row must yield None");
}

#[test]
fn fresh_row_with_empty_working_set_is_near_neutral() {
    let (_d, conn) = code_seed::open_db();
    let cfg = Config::defaults();
    let id = code_seed::seed_symbol(&conn, "demo", "a.rs", "a_run");
    let parts = prior_product(&conn, &cfg, id, &WorkingSet::default(), 0.0)
        .expect("prior_product")
        .expect("row exists");
    assert!(
        (parts.rank - 1.0).abs() < 1e-12,
        "zero median (unranked repo) must keep rank neutral, got {}",
        parts.rank
    );
    assert!(
        (parts.affinity - 1.0).abs() < 1e-12,
        "empty working set must keep affinity neutral"
    );
    assert!(
        (parts.feedback - 1.0).abs() < 1e-12,
        "no feedback rows must keep feedback neutral"
    );
    // Just-indexed row, zero accesses: activation ≈ exp(0) = 1.
    assert!(
        (parts.activation - 1.0).abs() < 1e-2,
        "fresh row activation must be near neutral, got {}",
        parts.activation
    );
    let product = parts.rank * parts.activation * parts.affinity * parts.feedback;
    assert!(
        (parts.final_score - product).abs() < 1e-12,
        "final_score must equal the prior product"
    );
}

#[test]
fn boosted_signals_raise_the_product() {
    let (_d, conn) = code_seed::open_db();
    let cfg = Config::defaults();
    let hot = code_seed::seed_symbol(&conn, "demo", "hot.rs", "hot_run");
    let cold = code_seed::seed_symbol(&conn, "demo", "cold.rs", "cold_run");
    conn.execute(
        "UPDATE code_symbols SET rank_score = 0.9, access_count = 40 WHERE id = ?1",
        [hot],
    )
    .expect("boost hot");
    conn.execute(
        "UPDATE code_symbols SET rank_score = 0.1 WHERE id = ?1",
        [cold],
    )
    .expect("set cold");
    conn.execute(
        "INSERT INTO code_feedback(repo, path, symbol, used_count, irrelevant_count) \
         VALUES ('demo', 'hot.rs', 'hot_run', 6, 0)",
        [],
    )
    .expect("seed feedback");

    let median = pool_median_rank(&conn, &[hot, cold]).expect("median");
    assert!(
        (median - 0.5).abs() < 1e-12,
        "median of 0.1/0.9 must be 0.5, got {median}"
    );

    let ws = WorkingSet::default();
    let h = prior_product(&conn, &cfg, hot, &ws, median)
        .expect("prior_product")
        .expect("hot row");
    let c = prior_product(&conn, &cfg, cold, &ws, median)
        .expect("prior_product")
        .expect("cold row");
    assert!(h.rank > 1.0, "above-median rank_score must boost");
    assert!(c.rank < h.rank, "below-median rank_score must boost less");
    assert!(h.activation > 1.0, "recent accesses must boost activation");
    assert!(h.feedback > 1.0, "positive feedback must boost");
    assert!(h.final_score > c.final_score);
}

/// cAST chunk rows inherit their PARENT's feedback: the CLI feedback path
/// records against the coalesced parent id, so the chunk's own `<name>#<n>`
/// symbol never owns a `code_feedback` row — the prior join resolves the
/// chunk's EFFECTIVE identity (the parent's symbol name) instead, so the
/// parent's feedback influences its chunks while they are scored
/// pre-coalesce.
#[test]
fn chunk_rows_inherit_parent_feedback() {
    use comemory::store::code_row::{self, CodeSymbolRow};

    let (_d, conn) = code_seed::open_db();
    let cfg = Config::defaults();
    let parent = code_seed::seed_symbol(&conn, "demo", "big.rs", "big_run");
    let chunk = code_row::insert(
        &conn,
        &CodeSymbolRow {
            repo: "demo",
            path: "big.rs",
            blob_oid: "oid",
            symbol: "big_run#1",
            kind: "function",
            lang: "rust",
            line_start: 11,
            line_end: 20,
            snippet: "chunk body",
            simhash: 0,
            parent_id: Some(parent),
        },
    )
    .expect("insert chunk row");
    // Feedback recorded under the parent's identity (what the CLI path
    // writes after coalescing).
    conn.execute(
        "INSERT INTO code_feedback(repo, path, symbol, used_count, irrelevant_count) \
         VALUES ('demo', 'big.rs', 'big_run', 6, 0)",
        [],
    )
    .expect("seed parent feedback");

    let ws = WorkingSet::default();
    let p = prior_product(&conn, &cfg, parent, &ws, 0.0)
        .expect("prior_product")
        .expect("parent row");
    let c = prior_product(&conn, &cfg, chunk, &ws, 0.0)
        .expect("prior_product")
        .expect("chunk row");
    assert!(p.feedback > 1.0, "parent feedback boosts the parent");
    assert!(
        (c.feedback - p.feedback).abs() < 1e-12,
        "chunk must inherit the parent's feedback boost, got chunk {} vs parent {}",
        c.feedback,
        p.feedback
    );
}

#[test]
fn pool_median_rank_dedups_files_and_skips_vanished_ids() {
    let (_d, conn) = code_seed::open_db();
    let a0 = code_seed::seed_symbol(&conn, "demo", "a.rs", "a_one");
    let a1 = code_seed::seed_symbol(&conn, "demo", "a.rs", "a_two");
    let b = code_seed::seed_symbol(&conn, "demo", "b.rs", "b_run");
    let c = code_seed::seed_symbol(&conn, "demo", "c.rs", "c_run");
    for (id, score) in [(a0, 0.2), (a1, 0.2), (b, 0.6), (c, 1.0)] {
        conn.execute(
            "UPDATE code_symbols SET rank_score = ?2 WHERE id = ?1",
            rusqlite::params![id, score],
        )
        .expect("set rank");
    }

    // Dedup by (repo, path): distinct file ranks are [0.2, 0.6, 1.0] →
    // median 0.6. Without the dedup the even-sized pool [0.2, 0.2, 0.6,
    // 1.0] would yield 0.4. The vanished id must be skipped silently.
    let median = pool_median_rank(&conn, &[a0, a1, b, c, 9_999]).expect("median");
    assert!((median - 0.6).abs() < 1e-12, "got {median}");

    let empty = pool_median_rank(&conn, &[]).expect("median of empty pool");
    assert_eq!(empty, 0.0, "empty pool maps to 0.0 (neutral rank prior)");
}
