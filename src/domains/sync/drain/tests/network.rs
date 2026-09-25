#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Network-state bookkeeping: a `401` suspends on the credential it saw, a
//! backoff lands inside its window, `Retry-After` gates even a manual run, and
//! a success clears everything.

use std::time::Duration;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::domains::sync::drain::network;
use crate::domains::sync::drain::transport::Failure;
use crate::store::sync_exchange::{ExchangeKey, ExchangeRow};

fn row() -> ExchangeRow {
    ExchangeRow::fresh(&ExchangeKey::new("http://127.0.0.1:9/api", "ws"))
}

fn retry_in(row: &ExchangeRow) -> Duration {
    let at = OffsetDateTime::parse(row.retry_at.as_deref().expect("retry_at"), &Rfc3339)
        .expect("rfc3339");
    Duration::try_from(at - OffsetDateTime::now_utc()).unwrap_or(Duration::ZERO)
}

#[test]
fn a_refused_credential_suspends_until_the_credential_changes() {
    let mut r = row();
    network::fail(&mut r, &Failure::Auth(401), "fp-old").expect("fail");
    assert_eq!(r.network_state, "auth_suspended");
    assert!(
        network::gated(&r, "fp-old", true).expect("gate"),
        "even a manual run waits"
    );
    assert!(
        !network::gated(&r, "fp-new", false).expect("gate"),
        "a new credential resumes"
    );
}

#[test]
fn consecutive_outages_widen_the_jittered_window() {
    let mut r = row();
    network::fail(&mut r, &Failure::Unavailable("HTTP 502".into()), "fp").expect("1");
    assert!(retry_in(&r) <= Duration::from_secs(2));
    network::fail(&mut r, &Failure::Unavailable("HTTP 502".into()), "fp").expect("2");
    assert_eq!(r.consecutive_failures, 2);
    assert!(retry_in(&r) <= Duration::from_secs(4));
}

#[test]
fn retry_after_gates_a_manual_run_and_an_outage_does_not() {
    let mut asked = row();
    network::fail(
        &mut asked,
        &Failure::RateLimited(Some(Duration::from_secs(30))),
        "fp",
    )
    .expect("429");
    assert!(
        network::gated(&asked, "fp", true).expect("manual"),
        "the upstream asked"
    );
    let mut outage = row();
    outage.consecutive_failures = 8;
    network::fail(&mut outage, &Failure::Unavailable("timeout".into()), "fp").expect("outage");
    assert!(
        retry_in(&outage) <= crate::domains::sync::drain::backoff::window(9),
        "the outage's retry lands inside its window"
    );
    // The jittered draw may be zero; gate on a retry that is still ahead.
    let ahead = OffsetDateTime::now_utc() + time::Duration::minutes(1);
    outage.retry_at = Some(ahead.format(&Rfc3339).expect("stamp"));
    assert!(
        network::gated(&outage, "fp", false).expect("unattended"),
        "an unattended pass honors an outage backoff"
    );
    assert!(
        !network::gated(&outage, "fp", true).expect("manual"),
        "a person may retry now"
    );
}

#[test]
fn a_success_clears_the_state_and_protocol_errors_do_not_gate() {
    let mut r = row();
    network::fail(&mut r, &Failure::Protocol("garbage".into()), "fp").expect("protocol");
    assert_eq!(r.network_state, "protocol_error");
    assert!(
        !network::gated(&r, "fp", false).expect("gate"),
        "the next pass renegotiates"
    );
    network::succeed(&mut r).expect("succeed");
    assert_eq!(r.network_state, "ok");
    assert_eq!(r.consecutive_failures, 0);
    assert!(r.retry_at.is_none() && r.last_error.is_none());
}
