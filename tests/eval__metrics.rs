//! Tests for [`comemory::eval::metrics`] — recall@k and MRR building
//! blocks plus the percentile bootstrap that brackets them.

use comemory::eval::bandit_rng::SplitMix64;
use comemory::eval::metrics::{BOOTSTRAP_ITERS, bootstrap_ci, first_hit_rank, recall_at_k};
use proptest::prelude::*;

/// A spread-out golden-shaped score vector: 8 hits, 2 misses.
fn mixed_scores() -> Vec<f64> {
    vec![1.0, 1.0, 1.0, 0.5, 1.0, 0.0, 1.0, 0.0, 1.0, 0.5]
}

/// Scores with pairwise-distinct values, so two resamples that differ at
/// all differ in their percentile means (recall values collide on a coarse
/// lattice and would blunt the divergence assertions below).
fn spread_scores() -> Vec<f64> {
    vec![0.13, 0.29, 0.41, 0.57, 0.68, 0.72, 0.85, 0.91, 0.96, 0.99]
}

/// Arithmetic mean, mirroring the point estimate the interval brackets.
fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

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

#[test]
fn bootstrap_ci_is_deterministic_for_a_fixed_seed() {
    let scores = spread_scores();
    let mut a = SplitMix64::new(42);
    let mut b = SplitMix64::new(42);
    let first = bootstrap_ci(&scores, BOOTSTRAP_ITERS, &mut a);
    let second = bootstrap_ci(&scores, BOOTSTRAP_ITERS, &mut b);
    assert_eq!(
        first, second,
        "same seed must reproduce the interval bit-for-bit"
    );

    let mut other = SplitMix64::new(43);
    let third = bootstrap_ci(&scores, BOOTSTRAP_ITERS, &mut other);
    assert_ne!(
        first, third,
        "a different seed must draw a different resample"
    );
}

#[test]
fn bootstrap_ci_brackets_the_sample_mean() {
    let scores = mixed_scores();
    let point = mean(&scores);
    let mut rng = SplitMix64::new(7);
    let (lo, hi) = bootstrap_ci(&scores, BOOTSTRAP_ITERS, &mut rng);
    assert!(
        lo <= point && point <= hi,
        "interval [{lo}, {hi}] must contain the sample mean {point}"
    );
    assert!(lo < hi, "a spread sample must produce a non-zero width");
    assert!(
        (0.0..=1.0).contains(&lo) && (0.0..=1.0).contains(&hi),
        "recall-shaped scores keep the interval in the unit range: [{lo}, {hi}]"
    );
}

#[test]
fn bootstrap_ci_collapses_on_degenerate_input() {
    let mut rng = SplitMix64::new(1);
    assert_eq!(
        bootstrap_ci(&[], BOOTSTRAP_ITERS, &mut rng),
        (0.0, 0.0),
        "an empty golden set has no point estimate to bracket"
    );
    assert_eq!(
        bootstrap_ci(&[0.75], BOOTSTRAP_ITERS, &mut rng),
        (0.75, 0.75),
        "a single query carries no spread"
    );
    assert_eq!(
        bootstrap_ci(&[1.0, 1.0, 1.0, 1.0], BOOTSTRAP_ITERS, &mut rng),
        (1.0, 1.0),
        "every resample of identical scores has the same mean"
    );
    assert_eq!(
        bootstrap_ci(&mixed_scores(), 0, &mut rng),
        (mean(&mixed_scores()), mean(&mixed_scores())),
        "zero iterations degrade to the point estimate"
    );
}

#[test]
fn bootstrap_ci_shares_one_rng_stream_across_calls() {
    // The runner draws recall then mrr from a single stream, so the pair of
    // intervals — not just each one — must replay from the seed alone.
    let scores = spread_scores();
    let mut rng = SplitMix64::new(99);
    let first = bootstrap_ci(&scores, BOOTSTRAP_ITERS, &mut rng);
    let second = bootstrap_ci(&scores, BOOTSTRAP_ITERS, &mut rng);
    assert_ne!(first, second, "the stream must advance between calls");

    let mut replay = SplitMix64::new(99);
    assert_eq!(bootstrap_ci(&scores, BOOTSTRAP_ITERS, &mut replay), first);
    assert_eq!(bootstrap_ci(&scores, BOOTSTRAP_ITERS, &mut replay), second);
}

#[test]
fn bootstrap_ci_leaves_the_stream_untouched_when_it_cannot_resample() {
    // Degenerate inputs short-circuit, so a golden set of one query does
    // not silently shift the draws of whatever interval comes next.
    let mut rng = SplitMix64::new(5);
    let _ = bootstrap_ci(&[0.5], BOOTSTRAP_ITERS, &mut rng);
    let _ = bootstrap_ci(&[], BOOTSTRAP_ITERS, &mut rng);
    let mut fresh = SplitMix64::new(5);
    assert_eq!(
        rng.next_u64(),
        fresh.next_u64(),
        "short-circuited calls must not consume draws"
    );
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
