#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirrors `src/config/activity.rs` — the `[activity]` defaults, the real
//! `config.toml` overlay, and the one refused value.
//!
//! The `COMEMORY_ACTIVITY_*` / `COMEMORY_ACTOR` env layer is covered in
//! `tests/cli__activity.rs` instead, against a real `comemory` process whose
//! environment the test sets on the child — stronger evidence than mutating
//! this process's own environment, and it needs no `unsafe`.

use comemory::config::file::Config;
use comemory::prelude::Result;
use tempfile::tempdir;

/// Write a real `config.toml` and overlay it, the same path
/// `cli::load_config` takes.
fn with_toml(body: &str) -> Result<Config> {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, body).unwrap();
    Config::defaults().with_file(&path)
}

#[test]
fn defaults_record_with_summaries_and_poll_twice_a_second() {
    let c = Config::defaults();
    assert!(c.activity.enabled, "the feed is on out of the box");
    assert!(c.activity.summaries);
    assert_eq!(c.activity.stream_poll_ms, 500);
    assert_eq!(c.activity.actor, None, "an actor is never inferred");
}

#[test]
fn the_file_overlay_turns_recording_and_summaries_off() {
    let c = with_toml(
        r"
[activity]
enabled = false
summaries = false
stream_poll_ms = 2000
",
    )
    .unwrap();
    assert!(!c.activity.enabled);
    assert!(!c.activity.summaries);
    assert_eq!(c.activity.stream_poll_ms, 2000);
}

#[test]
fn an_absent_key_leaves_the_default_standing() {
    let c = with_toml(
        r"
[activity]
summaries = false
",
    )
    .unwrap();
    assert!(c.activity.enabled, "only the named key is overlaid");
    assert!(!c.activity.summaries);
    assert_eq!(c.activity.stream_poll_ms, 500);
}

#[test]
fn actor_is_env_only_and_the_file_refuses_it() {
    let err = with_toml(
        r#"
[activity]
actor = "from-the-file"
"#,
    )
    .expect_err("actor is env-only, so the file key is refused");
    assert!(
        format!("{err}").contains("actor"),
        "the refusal names the offending key: {err}"
    );
}

#[test]
fn a_zero_poll_interval_is_refused_rather_than_clamped() {
    let err = with_toml(
        r"
[activity]
stream_poll_ms = 0
",
    )
    .expect_err("a zero interval would spin the stream task");
    let text = format!("{err}");
    assert!(text.contains("activity.stream_poll_ms"), "{text}");
    assert!(text.contains("COMEMORY_ACTIVITY_STREAM_POLL_MS"), "{text}");
}
