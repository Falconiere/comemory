#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Pure-logic tests for this module's own `Intent`/`action_of`; the real
//! repair/replace behavior is proven against real coordinators in
//! `tests/replica_daemon.rs` (#257).

use super::{Intent, action_of, not_ready};
use crate::domains::sync::daemon::supervisor::Kind;

#[test]
fn preflight_and_ensure_get_the_short_bound_repair_the_longer_one() {
    assert!(Intent::Preflight.bound() < Intent::Ensure.bound());
    assert_eq!(Intent::Ensure.bound(), Intent::Restart.bound());
    assert_eq!(Intent::Restart.bound(), Intent::Repair.bound());
}

#[test]
fn each_intent_names_a_distinct_action_except_preflight_and_ensure() {
    assert_eq!(action_of(Intent::Preflight), action_of(Intent::Ensure));
    assert_ne!(action_of(Intent::Restart), action_of(Intent::Repair));
    assert_ne!(action_of(Intent::Ensure), action_of(Intent::Restart));
}

#[test]
fn a_not_ready_error_carries_the_notes_an_installer_would_otherwise_lose() {
    let noted = not_ready(Kind::Process, vec!["a".into(), "b".into()], "e".into());
    assert_eq!(noted.error.as_deref(), Some("e [a; b]"));
    assert!(!noted.ready);
    let plain = not_ready(Kind::Process, Vec::new(), "e".into());
    assert_eq!(plain.error.as_deref(), Some("e"));
}
