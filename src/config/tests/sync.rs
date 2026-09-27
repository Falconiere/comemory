#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Duration parsing for `[sync]` knobs.

use comemory::config::sync::{PartialSyncConfig, SyncConfig, parse_duration};
use std::time::Duration;

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

#[test]
fn request_timeout_defaults_to_thirty_seconds_and_overlays() {
    let mut cfg = SyncConfig::defaults();
    assert_eq!(
        parse_duration(&cfg.request_timeout).expect("default"),
        Duration::from_secs(30)
    );
    let partial: PartialSyncConfig = toml::from_str("request_timeout = \"2s\"").expect("parse");
    cfg.apply(partial);
    assert_eq!(
        parse_duration(&cfg.request_timeout).expect("overlay"),
        Duration::from_secs(2)
    );
}

#[test]
fn pass_budget_and_max_request_bytes_overlay_from_the_file() {
    let mut cfg = SyncConfig::defaults();
    assert_eq!(
        parse_duration(&cfg.pass_budget).expect("default"),
        Duration::from_secs(30)
    );
    assert_eq!(cfg.max_request_bytes, 4 * 1024 * 1024);
    let partial: PartialSyncConfig =
        toml::from_str("pass_budget = \"1s\"\nmax_request_bytes = 262144").expect("parse");
    cfg.apply(partial);
    assert_eq!(
        parse_duration(&cfg.pass_budget).expect("overlay"),
        Duration::from_secs(1)
    );
    assert_eq!(cfg.max_request_bytes, 262_144);
}

#[test]
fn pause_wait_defaults_to_five_seconds_and_overlays() {
    let mut cfg = SyncConfig::defaults();
    assert_eq!(
        cfg.pause_wait_duration().expect("default"),
        Duration::from_secs(5)
    );
    let partial: PartialSyncConfig = toml::from_str("pause_wait = \"1s\"").expect("parse");
    cfg.apply(partial);
    assert_eq!(
        cfg.pause_wait_duration().expect("overlay"),
        Duration::from_secs(1)
    );
}

#[test]
fn a_zero_pause_wait_is_refused_rather_than_failing_busy_on_any_contention() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[sync]\npause_wait = \"0s\"\n").unwrap();

    let err = comemory::config::Config::defaults()
        .with_file(&path)
        .expect_err("a zero bound must not load");
    assert!(
        err.to_string().contains("pause_wait"),
        "the error must name the offending key: {err}"
    );
}

#[test]
fn milliseconds_read_as_milliseconds_not_minutes() {
    assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
    assert_eq!(parse_duration("5s").unwrap(), Duration::from_secs(5));
    assert_eq!(parse_duration("2m").unwrap(), Duration::from_mins(2));
    assert_eq!(parse_duration("1h").unwrap(), Duration::from_hours(1));
    assert_eq!(
        parse_duration("1500MS").unwrap(),
        Duration::from_millis(1500)
    );
}

#[test]
fn an_unknown_unit_is_refused_rather_than_guessed_from_its_first_letter() {
    for raw in [
        "5x", "5sec", "5secs", "5min", "5hours", "5 s", "5ms2", "5mss", "5",
    ] {
        let err = parse_duration(raw).expect_err(raw);
        assert!(
            err.to_string().contains(raw.trim()),
            "the error must quote the value: {raw} → {err}"
        );
    }
    let err = parse_duration("5sec").unwrap_err().to_string();
    assert!(
        err.contains("ms, s, m, h, or d"),
        "names the accepted units: {err}"
    );
}

#[test]
fn a_sub_second_pause_wait_loads_through_the_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[sync]\npause_wait = \"500ms\"\n").unwrap();

    let cfg = comemory::config::Config::defaults()
        .with_file(&path)
        .expect("a millisecond bound loads");
    assert_eq!(
        cfg.sync.pause_wait_duration().unwrap(),
        Duration::from_millis(500)
    );

    std::fs::write(&path, "[sync]\npause_wait = \"5sec\"\n").unwrap();
    let err = comemory::config::Config::defaults()
        .with_file(&path)
        .expect_err("an unknown unit must not load");
    assert!(err.to_string().contains("pause_wait"), "{err}");
}
