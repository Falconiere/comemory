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
}
