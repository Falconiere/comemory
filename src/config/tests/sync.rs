#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Duration parsing for `[sync]` knobs.

use comemory::config::sync::{SyncConfig, parse_duration};

#[test]
fn parse_compact_durations() {
    assert_eq!(parse_duration("30s").unwrap().as_secs(), 30);
    assert_eq!(parse_duration("5m").unwrap().as_secs(), 300);
    assert_eq!(parse_duration("1h").unwrap().as_secs(), 3600);
    assert_eq!(parse_duration("7d").unwrap().as_secs(), 604_800);
}

#[test]
fn defaults_round_trip() {
    let cfg = SyncConfig::defaults();
    assert!(!cfg.after_save);
    assert!(cfg.pull_before_context_after.is_empty());
    assert_eq!(cfg.daemon_interval, "5s");
    assert_eq!(
        cfg.daemon_interval_duration().unwrap(),
        std::time::Duration::from_secs(5)
    );
    assert!(cfg.allowlist_ttl_duration().is_ok());
}

#[test]
fn empty_pull_before_context_is_zero_duration() {
    let cfg = SyncConfig::defaults();
    assert_eq!(
        cfg.pull_before_context_after_duration().unwrap(),
        std::time::Duration::ZERO
    );
}
