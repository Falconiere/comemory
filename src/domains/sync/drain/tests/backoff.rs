#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The backoff windows the spec documents: 2 s, 4 s, … capped at five minutes,
//! full jitter inside them, and 1–8 s between in-pass retries.

use std::time::Duration;

use crate::domains::sync::drain::backoff;

#[test]
fn the_window_doubles_per_failure_and_caps_at_five_minutes() {
    assert_eq!(backoff::window(1), Duration::from_secs(2));
    assert_eq!(backoff::window(2), Duration::from_secs(4));
    assert_eq!(backoff::window(8), Duration::from_secs(256));
    assert_eq!(backoff::window(9), backoff::MAX);
    assert_eq!(backoff::window(40), backoff::MAX);
}

#[test]
fn the_delay_is_drawn_inside_its_window() {
    assert_eq!(backoff::after_failures(2, 0.0), Duration::ZERO);
    assert_eq!(backoff::after_failures(2, 1.0), Duration::from_secs(4));
    for _ in 0..200 {
        let d = backoff::after_failures(3, backoff::fraction());
        assert!(d <= Duration::from_secs(8), "{d:?}");
    }
}

#[test]
fn in_pass_retries_wait_between_one_and_eight_seconds() {
    assert_eq!(backoff::in_pass(0.0), Duration::from_secs(1));
    assert_eq!(backoff::in_pass(1.0), Duration::from_secs(8));
}
