#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for the two pure decisions an arm report makes: what a latency
//! distribution reads off a sample, and which verdict a paired interval earns.
//! The verdict is read off the interval, never the point estimate, so an arm
//! that looks better but overlaps zero must not read as an improvement.

use comemory::domains::learning::evaluation::benchmark_arm_report::{Verdict, latency};

#[test]
fn a_latency_distribution_is_ordered_and_covers_its_sample() {
    let l = latency(&[5, 1, 9, 3, 7]);
    assert_eq!(l.p50, 5);
    assert_eq!(l.max, 9);
    assert!(l.p50 <= l.p90 && l.p90 <= l.p95 && l.p95 <= l.max);
    assert_eq!(l.mean, 5.0);
}

#[test]
fn an_empty_sample_yields_zeros_rather_than_dividing_by_zero() {
    let l = latency(&[]);
    assert_eq!((l.p50, l.p90, l.p95, l.max), (0, 0, 0, 0));
    assert_eq!(l.mean, 0.0);
}

#[test]
fn a_single_sample_is_every_percentile() {
    let l = latency(&[42]);
    assert_eq!((l.p50, l.p90, l.p95, l.max), (42, 42, 42, 42));
    assert_eq!(l.mean, 42.0);
}

#[test]
fn the_verdict_vocabulary_serializes_lowercase_for_the_artifact() {
    for (verdict, expected) in [
        (Verdict::Baseline, "\"baseline\""),
        (Verdict::Improved, "\"improved\""),
        (Verdict::Regressed, "\"regressed\""),
        (Verdict::Neutral, "\"neutral\""),
        (Verdict::Inconclusive, "\"inconclusive\""),
    ] {
        assert_eq!(
            serde_json::to_string(&verdict).expect("serialize"),
            expected
        );
    }
}
