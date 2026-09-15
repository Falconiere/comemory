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
fn the_code_index_ships_on_by_default() {
    let cfg = SyncConfig::defaults();
    assert!(
        cfg.code_index,
        "the console's graph must fill in right after login"
    );
}

#[test]
fn empty_pull_before_context_is_zero_duration() {
    let cfg = SyncConfig::defaults();
    assert_eq!(
        cfg.pull_before_context_after_duration().unwrap(),
        std::time::Duration::ZERO
    );
}

#[test]
fn push_on_save_is_on_by_default_with_a_two_second_budget() {
    // Continuous sync stops depending on a resident process only if the
    // writing command itself pushes — so the default is on.
    let cfg = SyncConfig::defaults();
    assert!(cfg.push_on_save);
    assert_eq!(
        cfg.push_on_save_timeout_duration().unwrap(),
        std::time::Duration::from_secs(2)
    );
}

#[test]
fn a_config_file_can_turn_the_inline_push_off_and_widen_its_budget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[sync]\npush_on_save = false\npush_on_save_timeout = \"10s\"\n",
    )
    .unwrap();

    let cfg = comemory::config::Config::defaults()
        .with_file(&path)
        .expect("overlay");
    assert!(!cfg.sync.push_on_save);
    assert_eq!(
        cfg.sync.push_on_save_timeout_duration().unwrap(),
        std::time::Duration::from_secs(10)
    );
}

#[test]
fn a_zero_push_budget_is_refused_rather_than_silently_failing_every_push() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[sync]\npush_on_save_timeout = \"0s\"\n").unwrap();

    let err = comemory::config::Config::defaults()
        .with_file(&path)
        .expect_err("a zero budget must not load");
    let message = err.to_string();
    assert!(
        message.contains("push_on_save_timeout"),
        "the error must name the offending key: {message}"
    );
    assert!(
        message.contains("push_on_save = false"),
        "and point at the switch that actually disables it: {message}"
    );
}
