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
    assert!(cfg.after_save);
    assert!(cfg.allowlist_ttl_duration().is_ok());
}
