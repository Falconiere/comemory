#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `Retry-After` in both forms the header takes, the cap, and garbage.

use std::time::Duration;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc2822;

use crate::domains::sync::drain::retry_after::{self, CAP};

#[test]
fn delay_seconds_are_read_and_capped() {
    assert_eq!(retry_after::parse("2"), Some(Duration::from_secs(2)));
    assert_eq!(retry_after::parse(" 120 "), Some(Duration::from_mins(2)));
    assert_eq!(
        retry_after::parse("86400"),
        Some(CAP),
        "a day is read as the cap"
    );
}

#[test]
fn an_http_date_is_a_delay_from_now_and_a_past_date_is_zero() {
    let later = (OffsetDateTime::now_utc() + time::Duration::seconds(90))
        .format(&Rfc2822)
        .expect("format");
    let delay = retry_after::parse(&later).expect("date form");
    assert!(
        delay <= Duration::from_secs(90) && delay >= Duration::from_secs(85),
        "{delay:?}"
    );
    assert_eq!(
        retry_after::parse("Sun, 06 Nov 1994 08:49:37 GMT"),
        Some(Duration::ZERO)
    );
}

#[test]
fn garbage_is_none() {
    assert_eq!(retry_after::parse("soon"), None);
    assert_eq!(retry_after::parse("-5"), None);
}
