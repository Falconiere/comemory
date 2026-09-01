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
//! round-trip via COMEMORY_EMBED_HINT, the v0.2 JSON report shape
//! (data_dir, db_writable, sqlite_vec_loaded), and the console-compat
//! `checks: Vec<Check>` array (spec AC-24..AC-28) — every scenario below
//! drives the real binary against a real markdown + SQLite corpus, a real
//! git repo indexed through `comemory index-code`, and a real
//! `printf`-backed `COMEMORY_EMBED_CMD` (no mocks, Binding Rule 9).

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use comemory::store::migrate::CURRENT_VERSION;
use tempfile::TempDir;

/// A real `sh -c`-executable command that emits a valid
/// `{"embedding":[..]}` payload. Drains stdin with `cat >/dev/null` first
/// (rather than a bare `printf`, which never reads its stdin at all) so the
/// parent's query write cannot race the child's exit and surface a spurious
/// broken-pipe failure under load.
const REAL_EMBED_CMD: &str = r#"cat >/dev/null; printf '{"embedding":[0.1,0.2,0.3]}'"#;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Run `doctor --json` and parse its stdout, asserting the process exits 0
/// — every scenario below (mirror drift, an unset embed command, a missing
/// repo root) is a `"warn"`, never a hard failure.
fn doctor_json(home: &TempDir) -> serde_json::Value {
    let assertion = bin(home).args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("doctor --json parses")
}

/// The single check named `name` in a `doctor --json` report, panicking
/// (with the full array) if it is missing.
fn find_check<'a>(report: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    report["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .find(|c| c["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("no check named {name:?} in {report}"))
}

/// The one live (non-hidden) markdown file under `<data_dir>/memories/`.
fn find_memory_md(data_dir: &Path) -> PathBuf {
    std::fs::read_dir(data_dir.join("memories"))
        .expect("read memories dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.extension().is_some_and(|ext| ext == "md")
                && !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .expect("exactly one memory markdown file")
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

/// AC-24: on a healthy real data dir (two real `comemory save`s, a real
/// embed command configured), `doctor --json` returns at least 10 checks,
/// every one `"ok"`, and every original scalar field is still present.
#[test]
fn doctor_reports_at_least_ten_healthy_checks_on_a_real_corpus() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["save", "doctor healthy corpus body one", "--kind", "note"])
        .assert()
        .success();
    bin(&home)
        .args(["save", "doctor healthy corpus body two", "--kind", "note"])
        .assert()
        .success();

    let mut c = bin(&home);
    c.env("COMEMORY_EMBED_CMD", REAL_EMBED_CMD);
    let assertion = c.args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");

    for field in [
        "data_dir",
        "db_writable",
        "schema_version",
        "sqlite_vec_loaded",
        "embed_hint",
        "unknown_migration_keys",
    ] {
        assert!(
            v.get(field).is_some(),
            "missing original field {field}: {v}"
        );
    }

    let checks = v["checks"].as_array().expect("checks array");
    assert!(
        checks.len() >= 10,
        "expected at least 10 checks, got {}: {checks:?}",
        checks.len()
    );
    for c in checks {
        assert_eq!(
            c["status"].as_str(),
            Some("ok"),
            "every check must be ok on a healthy corpus: {c}"
        );
    }
}

/// AC-25: editing a saved memory's markdown body so its
/// `sha256(body.trim_end())` no longer matches its `memories.content_hash`
/// row makes the mirror-parity check `"warn"` with `mirror_drift == 1`, and
/// the process still exits 0.
#[test]
fn doctor_mirror_drift_warns_and_still_exits_zero() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["save", "doctor mirror parity body", "--kind", "note"])
        .assert()
        .success();

    let data_dir = home.path().join(".comemory");
    let md_path = find_memory_md(&data_dir);
    let mut body = std::fs::read_to_string(&md_path).expect("read markdown");
    body.push_str("\nan edit that changes the body hash\n");
    std::fs::write(&md_path, body).expect("rewrite markdown");

    let v = doctor_json(&home);
    assert_eq!(v["mirror_drift"].as_u64(), Some(1), "report: {v}");
    assert_eq!(
        find_check(&v, "mirror parity")["status"].as_str(),
        Some("warn")
    );
}

/// AC-26: a real `COMEMORY_EMBED_CMD` makes the embed check `"ok"` with a
/// non-null `embed_probe_ms`; unset, it is `"warn"` with a null
/// `embed_probe_ms` — and the command exits 0 either way.
#[test]
fn doctor_embed_probe_ok_when_set_warn_when_unset() {
    let home = TempDir::new().expect("tempdir");

    let mut c = bin(&home);
    c.env("COMEMORY_EMBED_CMD", REAL_EMBED_CMD);
    let assertion = c.args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert!(
        v["embed_probe_ms"].is_number(),
        "embed_probe_ms must be non-null when the probe succeeds: {v}"
    );
    assert_eq!(
        find_check(&v, "embed command")["status"].as_str(),
        Some("ok")
    );

    let v_unset = doctor_json(&home);
    assert!(
        v_unset["embed_probe_ms"].is_null(),
        "embed_probe_ms must be null when COMEMORY_EMBED_CMD is unset: {v_unset}"
    );
    assert_eq!(
        find_check(&v_unset, "embed command")["status"].as_str(),
        Some("warn")
    );
}

/// AC-27: the tokenizer check is `"ok"` on a database opened through
/// `store::connection::open`, and the vec-dimension check reports the real
/// `memory_vec` / `code_vec` dims (1024 / 768) read from `schema_meta`.
#[test]
fn doctor_tokenizer_ok_and_vec_dims_are_1024_and_768() {
    let home = TempDir::new().expect("tempdir");
    let v = doctor_json(&home);
    assert_eq!(
        v["tokenizer_registered"].as_bool(),
        Some(true),
        "report: {v}"
    );
    assert_eq!(v["memory_vec_dim"].as_u64(), Some(1024), "report: {v}");
    assert_eq!(v["code_vec_dim"].as_u64(), Some(768), "report: {v}");
    assert_eq!(
        find_check(&v, "fts5 tokenizer")["status"].as_str(),
        Some("ok")
    );
    assert_eq!(find_check(&v, "sqlite-vec")["status"].as_str(), Some("ok"));
}

/// AC-28: a `repo_marker.root_path` whose directory no longer exists on
/// disk makes the repo-roots check `"warn"` with
/// `repo_roots_ok < repo_roots_total`.
#[test]
fn doctor_repo_roots_warns_when_a_root_no_longer_exists() {
    let home = TempDir::new().expect("tempdir");
    let repo = home.path().join("sample-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("src.rs", "fn main() {}\n")], "init");

    bin(&home)
        .args([
            "index-code",
            "--repo",
            "sample",
            "--path",
            repo.to_str().expect("utf8 repo path"),
        ])
        .assert()
        .success();

    std::fs::remove_dir_all(&repo).expect("remove indexed repo's working tree");

    let v = doctor_json(&home);
    let ok_count = v["repo_roots_ok"].as_u64().expect("repo_roots_ok");
    let total = v["repo_roots_total"].as_u64().expect("repo_roots_total");
    assert!(
        ok_count < total,
        "expected repo_roots_ok < repo_roots_total, got {ok_count}/{total}: {v}"
    );
    assert_eq!(
        find_check(&v, "repo roots")["status"].as_str(),
        Some("warn")
    );
}
