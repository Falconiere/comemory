use comemory::retrieval::score::*;
use proptest::prelude::*;

const CLAMP: (f64, f64) = (0.5, 2.0);

#[test]
fn fresh_memory_is_neutral() {
    // n=1 (created counts as first access), same-day: activation 0 → boost 1.0
    let a = activation(0, 0.0, 0.5); // access_count 0 is floored to 1
    assert_eq!(a, 0.0);
    assert_eq!(activation_boost(a, CLAMP), 1.0);
}

#[test]
fn zero_feedback_is_neutral() {
    let b = beta_feedback(0, 0);
    assert_eq!(b, 0.25);
    assert_eq!(feedback_boost(b, CLAMP), 1.0);
}

#[test]
fn quality_three_is_neutral() {
    assert_eq!(quality_boost(3, CLAMP), 1.0);
}

#[test]
fn bounded_boost_clamps_and_neutralizes_non_finite() {
    assert_eq!(bounded_boost(1.3, CLAMP), 1.3);
    assert_eq!(bounded_boost(100.0, CLAMP), CLAMP.1);
    assert_eq!(bounded_boost(0.001, CLAMP), CLAMP.0);
    // non-finite input → neutral 1.0, itself clamped
    assert_eq!(bounded_boost(f64::NAN, CLAMP), 1.0);
    assert_eq!(bounded_boost(f64::INFINITY, CLAMP), 1.0);
    assert_eq!(bounded_boost(f64::NAN, (1.5, 2.0)), 1.5);
}

#[test]
fn old_unaccessed_memory_sinks_below_threshold() {
    // single access 90 days ago ≈ −2.26 < default prune floor −2.0
    let a = activation(1, 90.0, 0.5);
    assert!(a < -2.0, "got {a}");
}

#[test]
fn days_since_counts_elapsed_days() {
    let now = time::macros::datetime!(2026-06-09 00:00:00 UTC);
    let d = days_since("2026-06-01T00:00:00Z", now);
    assert!((d - 8.0).abs() < 1e-9, "got {d}");
}

#[test]
fn days_since_floors_future_timestamps_at_zero() {
    let now = time::macros::datetime!(2026-06-09 00:00:00 UTC);
    assert_eq!(days_since("2026-07-01T00:00:00Z", now), 0.0);
}

#[test]
fn days_since_treats_malformed_timestamp_as_fresh() {
    let now = time::macros::datetime!(2026-06-09 00:00:00 UTC);
    assert_eq!(days_since("not-a-timestamp", now), 0.0);
}

#[test]
fn min_max_normalize_maps_pool_to_unit_interval() {
    assert_eq!(min_max_normalize(&[2.0, 4.0, 3.0]), vec![0.0, 1.0, 0.5]);
}

#[test]
fn min_max_normalize_degenerate_pools_are_all_ones() {
    assert_eq!(min_max_normalize(&[7.0, 7.0]), vec![1.0, 1.0]);
    assert_eq!(min_max_normalize(&[f64::NAN, 1.0]), vec![1.0, 1.0]);
    assert_eq!(
        min_max_normalize(&[1.0, 2.0, f64::NAN]),
        vec![1.0, 1.0, 1.0]
    );
    assert_eq!(min_max_normalize(&[]), Vec::<f64>::new());
}

#[test]
fn max_normalize_preserves_within_pool_ratios() {
    assert_eq!(max_normalize(&[2.0, 8.0, 4.0]), vec![0.25, 1.0, 0.5]);
}

#[test]
fn max_normalize_degenerate_pools_are_all_ones() {
    assert_eq!(max_normalize(&[7.0, 7.0]), vec![1.0, 1.0]);
    // all-non-positive → degenerate
    assert_eq!(max_normalize(&[-8.0, -2.0]), vec![1.0, 1.0]);
    assert_eq!(max_normalize(&[f64::NAN, 1.0]), vec![1.0, 1.0]);
    assert_eq!(max_normalize(&[]), Vec::<f64>::new());
}

#[test]
fn max_normalize_clamps_stray_negatives_in_positive_pools() {
    assert_eq!(max_normalize(&[-1.0, 2.0]), vec![0.0, 1.0]);
}

#[test]
fn median_rank_takes_the_middle_of_an_odd_pool() {
    // Unsorted input: the helper sorts in place before picking.
    let mut v = vec![5.0, 1.0, 3.0];
    assert_eq!(median_rank(&mut v), 3.0);
    assert_eq!(v, vec![1.0, 3.0, 5.0], "input is sorted in place");
}

#[test]
fn median_rank_averages_the_middle_two_of_an_even_pool() {
    let mut v = vec![4.0, 1.0, 3.0, 2.0];
    assert_eq!(median_rank(&mut v), 2.5);
}

#[test]
fn median_rank_of_an_empty_pool_is_zero() {
    // 0.0 is the value rank_boost reads as "nothing ranked yet".
    assert_eq!(median_rank(&mut []), 0.0);
    assert_eq!(rank_boost(1.0, median_rank(&mut []), 0.2, CLAMP), 1.0);
}

#[test]
fn rank_boost_is_neutral_without_a_positive_median() {
    // Every score still at the 0.0 column default → exactly 1.0, even for
    // a candidate whose own raw score is high. The neutral value is
    // returned unclamped, so a clamp excluding 1.0 does not shift it.
    assert_eq!(rank_boost(0.0, 0.0, 0.2, CLAMP), 1.0);
    assert_eq!(rank_boost(9.9, 0.0, 0.2, CLAMP), 1.0);
    assert_eq!(rank_boost(1.0, -1.0, 0.2, CLAMP), 1.0);
    assert_eq!(rank_boost(1.0, 0.0, 0.2, (1.5, 2.0)), 1.0);
}

#[test]
fn rank_boost_at_the_median_is_the_uniform_pool_multiplier() {
    // raw == median > 0 → 1 + scale·ln 2, the value every candidate gets
    // when a recompute ran over an edge-free (uniform PageRank) corpus.
    let at_median = rank_boost(0.25, 0.25, 0.2, CLAMP);
    assert!(
        (at_median - (1.0 + 0.2 * 2.0f64.ln())).abs() < 1e-12,
        "got {at_median}"
    );
    assert!(
        (at_median - 1.138_629_436_1).abs() < 1e-9,
        "got {at_median}"
    );
}

#[test]
fn rank_boost_is_monotone_and_clamped() {
    let low = rank_boost(0.1, 1.0, 0.2, CLAMP);
    let high = rank_boost(10.0, 1.0, 0.2, CLAMP);
    assert!(low < high, "{low} !< {high}");
    // A negative raw score floors at 0 → ln(1) = 0 → exactly 1.0.
    assert_eq!(rank_boost(-5.0, 1.0, 0.2, CLAMP), 1.0);
    // Both clamp bounds bind.
    assert_eq!(rank_boost(1e300, 1.0, 50.0, CLAMP), CLAMP.1);
    assert_eq!(rank_boost(1.0, 1.0, -50.0, CLAMP), CLAMP.0);
}

#[test]
fn rank_boost_scale_is_a_parameter_not_a_constant() {
    // The two call sites (code_prior::RANK_SCALE, rerank::MEMORY_RANK_SCALE)
    // set their own slope over one shared curve.
    let gentle = rank_boost(4.0, 1.0, 0.2, CLAMP);
    let steep = rank_boost(4.0, 1.0, 0.4, CLAMP);
    assert!(gentle < steep, "{gentle} !< {steep}");
    // A zero scale collapses the curve to neutral for any raw score.
    assert_eq!(rank_boost(4.0, 1.0, 0.0, CLAMP), 1.0);
}

proptest! {
    #[test]
    fn activation_monotone_in_count(n in 1u64..10_000, days in 0.0f64..3650.0) {
        prop_assert!(activation(n + 1, days, 0.5) >= activation(n, days, 0.5));
    }

    #[test]
    fn activation_decays_with_time(n in 1u64..10_000, days in 0.0f64..3650.0) {
        prop_assert!(activation(n, days + 1.0, 0.5) <= activation(n, days, 0.5));
    }

    #[test]
    fn irrelevant_votes_never_raise_feedback(u in 0u64..1000, i in 0u64..1000) {
        prop_assert!(beta_feedback(u, i + 1) <= beta_feedback(u, i));
    }

    #[test]
    fn boosts_always_within_clamp(a in -100.0f64..100.0, b in 0.0f64..1.0, q in 1u8..=5) {
        for v in [activation_boost(a, CLAMP), feedback_boost(b, CLAMP), quality_boost(q, CLAMP)] {
            prop_assert!(v.is_finite());
            prop_assert!((CLAMP.0..=CLAMP.1).contains(&v));
        }
    }

    #[test]
    fn no_nan_ever(n in 0u64..u64::MAX, days in -10.0f64..1.0e9, d in 0.0f64..10.0) {
        prop_assert!(activation(n, days, d).is_finite());
    }

    #[test]
    fn quality_boost_monotone_in_quality(q in 1u8..5) {
        prop_assert!(quality_boost(q + 1, CLAMP) >= quality_boost(q, CLAMP));
    }

    #[test]
    fn feedback_boost_monotone_in_beta(b in 0.0f64..1.0, delta in 0.0f64..1.0) {
        prop_assert!(feedback_boost(b + delta, CLAMP) >= feedback_boost(b, CLAMP));
    }

    #[test]
    fn activation_boost_monotone_in_activation(a in -100.0f64..100.0, delta in 0.0f64..100.0) {
        prop_assert!(activation_boost(a + delta, CLAMP) >= activation_boost(a, CLAMP));
    }

    #[test]
    fn used_votes_never_lower_feedback(u in 0u64..1000, i in 0u64..1000) {
        prop_assert!(beta_feedback(u + 1, i) >= beta_feedback(u, i));
    }

    #[test]
    fn rank_boost_always_within_clamp(
        raw in -1.0e6f64..1.0e6, median in -1.0e6f64..1.0e6, scale in 0.0f64..5.0
    ) {
        let v = rank_boost(raw, median, scale, CLAMP);
        prop_assert!(v.is_finite());
        prop_assert!((CLAMP.0..=CLAMP.1).contains(&v));
    }

    #[test]
    fn rank_boost_monotone_in_raw_score(
        raw in 0.0f64..1.0e3, delta in 0.0f64..1.0e3, median in 1.0e-3f64..1.0e3
    ) {
        prop_assert!(rank_boost(raw + delta, median, 0.2, CLAMP)
                  >= rank_boost(raw, median, 0.2, CLAMP));
    }

    #[test]
    fn median_rank_lands_between_the_extremes(v in prop::collection::vec(-1.0e6f64..1.0e6, 1..64)) {
        let mut values = v.clone();
        let m = median_rank(&mut values);
        let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        prop_assert!((lo..=hi).contains(&m), "median {m} outside [{lo}, {hi}]");
    }
}
