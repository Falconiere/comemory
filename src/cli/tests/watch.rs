#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! `comemory watch`'s reconnect schedule — the part of the loop that needs no
//! socket to be worth pinning.

use std::time::Duration;

use comemory::cli::watch::backoff_delay;

#[test]
fn backoff_grows_to_a_thirty_second_ceiling_and_never_drops_below_a_second() {
    // Full jitter: the floor is the minimum wait, the ceiling doubles per
    // attempt until it caps. A client that reconnects instantly in a loop is
    // what this schedule exists to prevent.
    assert_eq!(backoff_delay(0, 0.0), Duration::from_secs(1));
    assert_eq!(backoff_delay(0, 1.0), Duration::from_secs(1));
    assert_eq!(backoff_delay(3, 1.0), Duration::from_secs(8));
    assert_eq!(backoff_delay(20, 1.0), Duration::from_secs(30));
    assert_eq!(backoff_delay(20, 0.0), Duration::from_secs(1));
}

#[test]
fn a_fraction_outside_zero_to_one_cannot_push_the_delay_out_of_range() {
    // The fraction comes from /dev/urandom; clamping is what keeps a bad read
    // from turning into a negative or unbounded sleep.
    assert_eq!(backoff_delay(5, -1.0), Duration::from_secs(1));
    assert_eq!(backoff_delay(5, 2.0), backoff_delay(5, 1.0));
}

#[test]
fn the_midpoint_of_a_capped_window_is_halfway_between_floor_and_ceiling() {
    assert_eq!(backoff_delay(20, 0.5), Duration::from_millis(15_500));
}
