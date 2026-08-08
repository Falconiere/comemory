#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory doctor`.
//!
//! Covers schema_version (pinned to `migrate::CURRENT_VERSION`), embed_hint
//! round-trip via COMEMORY_EMBED_HINT, and the v0.2 JSON report shape
//! (data_dir, db_writable, sqlite_vec_loaded).

use assert_cmd::Command;
use comemory::store::migrate::CURRENT_VERSION;
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

#[test]
fn doctor_reports_current_schema_version_on_fresh_dir() {
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home).arg("doctor").assert().success();
    let out = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        out.contains("schema_version") && out.contains(&format!(": {CURRENT_VERSION}")),
        "doctor should report schema_version {CURRENT_VERSION} on a fresh dir: {out:?}"
    );
}

#[test]
fn doctor_json_emits_v2_report_shape() {
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home).args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert!(v["data_dir"].is_string());
    assert_eq!(v["db_writable"].as_bool(), Some(true));
    assert_eq!(v["schema_version"].as_str(), Some(CURRENT_VERSION));
    assert_eq!(v["sqlite_vec_loaded"].as_bool(), Some(true));
    // embed_hint must be present (null when not set).
    assert!(
        v.get("embed_hint").is_some(),
        "embed_hint field must exist in JSON"
    );
}

#[test]
fn doctor_schema_version_persists_after_save() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["save", "doctor save body", "--kind", "note"])
        .assert()
        .success();
    let assertion = bin(&home).args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(v["schema_version"].as_str(), Some(CURRENT_VERSION));
}

#[test]
fn doctor_embed_hint_round_trips_via_env_var() {
    let home = TempDir::new().expect("tempdir");
    let mut c = bin(&home);
    c.env("COMEMORY_EMBED_HINT", "ollama:nomic-embed-text");
    let assertion = c.args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(
        v["embed_hint"].as_str(),
        Some("ollama:nomic-embed-text"),
        "embed_hint must round-trip from env var; got: {v}"
    );

    // TTY output must also contain the hint.
    let mut c2 = bin(&home);
    c2.env("COMEMORY_EMBED_HINT", "ollama:nomic-embed-text");
    let tty = c2.arg("doctor").assert().success();
    let tty_out = String::from_utf8(tty.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        tty_out.contains("ollama:nomic-embed-text"),
        "TTY output must contain embed_hint; got: {tty_out:?}"
    );
}

/// `embed_hint` set in config.toml (the file layer) must appear in the doctor
/// report even when the env var is absent. This validates that `doctor` honours
/// the full defaults → file → env layering order rather than skipping the file.
#[test]
fn doctor_embed_hint_round_trips_via_config_file() {
    let home = TempDir::new().expect("tempdir");
    // Bootstrap the data dir so config.toml has a place to live.
    bin(&home).args(["doctor"]).assert().success();

    // Write a minimal config.toml under the data dir.
    let data_dir = home.path().join(".comemory");
    let config_path = data_dir.join("config.toml");
    std::fs::write(&config_path, "embed_hint = \"file-layer-embedder\"\n")
        .expect("write config.toml");

    // Run doctor without the env var so the value must come from the file.
    let assertion = bin(&home).args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(
        v["embed_hint"].as_str(),
        Some("file-layer-embedder"),
        "embed_hint from config.toml must reach the doctor report; got: {v}"
    );
}

/// Insert a bogus `0014_future` marker directly into a real, fully-migrated
/// v13 database — simulating a database written by a newer comemory. A
/// plain `rusqlite::Connection::open` is used rather than
/// `comemory::store::connection::open`, since the latter would itself run
/// `preflight`/`migrate::run` again on the very database this test is
/// about to make forward-incompatible.
fn inject_future_marker(db_path: &std::path::Path) {
    let conn = rusqlite::Connection::open(db_path).expect("open db to inject marker");
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES('0014_future', '1')",
        [],
    )
    .expect("seed future marker");
}

/// `doctor` is the one command whose job is to explain a broken state: on a
/// database written by a newer comemory it must fall back to a read-only
/// probe and report the unknown key (exit 0), while every other command is
/// refused identically to `Error::Migration`'s AC-12 exit-70 contract.
#[test]
fn doctor_falls_back_on_a_newer_db_while_every_other_command_still_exits_70() {
    let home = TempDir::new().expect("tempdir");
    // Bootstrap a real, fully-migrated v13 database.
    bin(&home).arg("doctor").assert().success();
    let db_path = home.path().join(".comemory").join("comemory.db");
    inject_future_marker(&db_path);

    // `doctor` (TTY) must exit 0 and name the unknown key.
    let assertion = bin(&home).arg("doctor").assert().success();
    let out = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        out.contains("0014_future"),
        "doctor TTY output must name the unknown key, got: {out:?}"
    );

    // `doctor --json` must exit 0 and carry the key in the JSON report.
    let assertion = bin(&home).args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(
        v["unknown_migration_keys"].as_array().map(Vec::len),
        Some(1),
        "unknown_migration_keys must carry exactly one entry, got: {v}"
    );
    assert_eq!(
        v["unknown_migration_keys"][0].as_str(),
        Some("0014_future"),
        "unknown_migration_keys must name the exact unknown key, got: {v}"
    );
    assert_eq!(v["db_writable"].as_bool(), Some(true));

    // Every other command against the same DB must still exit 70, per AC-12.
    for args in [
        vec!["list"],
        vec!["search", "anything"],
        vec!["save", "body", "--kind", "note"],
    ] {
        let assertion = bin(&home).args(&args).assert().code(70);
        let stderr = String::from_utf8(assertion.get_output().stderr.clone()).expect("utf8");
        assert!(
            stderr.contains("0014_future"),
            "{args:?} must surface the same forward-compat refusal, got: {stderr:?}"
        );
    }
}

/// Env var must override config.toml when both are set (env wins in the
/// defaults → file → env layering order).
#[test]
fn doctor_embed_hint_env_overrides_config_file() {
    let home = TempDir::new().expect("tempdir");
    bin(&home).args(["doctor"]).assert().success();

    let data_dir = home.path().join(".comemory");
    std::fs::write(
        data_dir.join("config.toml"),
        "embed_hint = \"file-value\"\n",
    )
    .expect("write config.toml");

    let mut c = bin(&home);
    c.env("COMEMORY_EMBED_HINT", "env-value");
    let assertion = c.args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(
        v["embed_hint"].as_str(),
        Some("env-value"),
        "env var must override config.toml embed_hint; got: {v}"
    );
}
