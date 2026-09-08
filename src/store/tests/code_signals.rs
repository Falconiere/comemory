#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/code_signals.rs` — real `code_symbols` +
//! `code_feedback` rows, no mocks. The exhaustive prior-math coverage lives
//! in `src/retrieval/tests/code_prior.rs` (through the `code_prior`
//! re-export); this file pins the SQL/row-mapping contract directly.

use crate::test_common::code_seed;
use comemory::store::code_signals::{signals, signals_batch};

#[test]
fn signals_is_none_for_vanished_symbol() {
    let (_d, conn) = code_seed::open_db();
    assert!(signals(&conn, 9_999).expect("query").is_none());
}

#[test]
fn signals_returns_seeded_row_with_neutral_feedback() {
    let (_d, conn) = code_seed::open_db();
    let id = code_seed::seed_symbol(&conn, "demo", "a.rs", "a_fn");
    let sig = signals(&conn, id).expect("query").expect("row present");
    assert_eq!(sig.repo, "demo");
    assert_eq!(sig.path, "a.rs");
    assert_eq!(sig.symbol, "a_fn");
    assert_eq!(sig.used, 0);
    assert_eq!(sig.irrelevant, 0);
    assert_eq!(sig.parent_id, None);
}

#[test]
fn signals_batch_matches_single_row_fetch_and_skips_vanished() {
    let (_d, conn) = code_seed::open_db();
    let a = code_seed::seed_symbol(&conn, "demo", "a.rs", "a_fn");
    let b = code_seed::seed_symbol(&conn, "demo", "b.rs", "b_fn");
    conn.execute(
        "INSERT INTO code_feedback(repo, path, symbol, used_count, irrelevant_count) \
         VALUES ('demo', 'a.rs', 'a_fn', 3, 1)",
        [],
    )
    .expect("seed feedback");

    let map = signals_batch(&conn, &[a, b, 9_999]).expect("batch");
    assert_eq!(map.len(), 2, "vanished id must be skipped, not error");

    let one = signals(&conn, a).expect("query").expect("row");
    let batched = map.get(&a).expect("present in batch");
    assert_eq!(batched.used, one.used);
    assert_eq!(batched.irrelevant, one.irrelevant);
    assert_eq!(batched.used, 3);
    assert_eq!(batched.irrelevant, 1);
}

#[test]
fn signals_batch_with_empty_ids_is_empty() {
    let (_d, conn) = code_seed::open_db();
    let map = signals_batch(&conn, &[]).expect("batch");
    assert!(map.is_empty());
}
