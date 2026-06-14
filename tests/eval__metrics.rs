//! Tests for [`comemory::eval::metrics`] — recall@k and MRR building blocks.

use comemory::eval::metrics::{first_hit_rank, recall_at_k};
use proptest::prelude::*;

#[test]
fn recall_at_k_counts_relevant_in_top_k() {
    let relevant = vec!["a".to_string(), "b".to_string()];
    let returned = vec!["x".to_string(), "a".to_string(), "b".to_string()];
    assert_eq!(recall_at_k(&relevant, &returned, 2), 0.5); // only "a" in top-2
    assert_eq!(recall_at_k(&relevant, &returned, 3), 1.0);
    assert_eq!(recall_at_k(&relevant, &[], 3), 0.0);
    assert_eq!(recall_at_k(&[], &returned, 3), 0.0); // degenerate: no relevant
}

#[test]
fn first_hit_rank_is_one_based() {
    let relevant = vec!["b".to_string()];
    let returned = vec!["x".to_string(), "b".to_string()];
    assert_eq!(first_hit_rank(&relevant, &returned), Some(2));
    assert_eq!(first_hit_rank(&relevant, &["x".to_string()]), None);
}

#[test]
fn first_hit_rank_prefers_earliest_relevant() {
    let relevant = vec!["a".to_string(), "b".to_string()];
    let returned = vec!["b".to_string(), "a".to_string()];
    assert_eq!(first_hit_rank(&relevant, &returned), Some(1));
}

#[test]
fn recall_at_k_ignores_hits_beyond_k() {
    let relevant = vec!["a".to_string()];
    let returned = vec!["x".to_string(), "y".to_string(), "a".to_string()];
    assert_eq!(recall_at_k(&relevant, &returned, 2), 0.0);
}

proptest! {
    #[test]
    fn recall_in_unit_interval_and_monotone_in_k(
        relevant in proptest::collection::vec("[a-c]{1,2}", 0..6),
        returned in proptest::collection::vec("[a-c]{1,2}", 0..6),
        k in 0usize..8,
    ) {
        let r_k = recall_at_k(&relevant, &returned, k);
        let r_k1 = recall_at_k(&relevant, &returned, k + 1);
        prop_assert!((0.0..=1.0).contains(&r_k), "recall@{k} out of range: {r_k}");
        prop_assert!(r_k1 >= r_k, "recall must be non-decreasing in k: {r_k1} < {r_k}");
    }
}
