//! Real-binary installation preview and offline asset distribution checks.
use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn installation_preview_does_not_write_or_require_a_host() {
    let home = tempfile::tempdir().unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("comemory"))
        .args(["install", "claude", "--dry-run", "--json", "--data-dir"])
        .arg(home.path().join("data"))
        .arg("--config-dir")
        .arg(home.path().join("host"))
        .assert()
        .success()
        .stdout(contains("comemory@comemory"));
    assert!(!home.path().join("data").exists());
    assert!(!home.path().join("host").exists());
}

/// AC-7: a dry-run install reports where `.mcp.json` would land under the
/// plugin root, without requiring the real `claude` CLI or writing anything.
/// The real write (and the exact `command`/`args` it carries for the test
/// binary) is exercised end to end by `scripts/test-agent-install.sh`, the
/// only journey that has the host CLI to install against.
#[test]
fn install_writes_mcp_manifest() {
    let home = tempfile::tempdir().unwrap();
    let assert = Command::new(assert_cmd::cargo::cargo_bin!("comemory"))
        .args(["install", "claude", "--dry-run", "--json", "--data-dir"])
        .arg(home.path().join("data"))
        .arg("--config-dir")
        .arg(home.path().join("host"))
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let manifest = report["mcp_manifest"].as_str().unwrap();
    let expected = home
        .path()
        .join("data")
        .join("integrations")
        .join(env!("CARGO_PKG_VERSION"))
        .join("plugins")
        .join("comemory")
        .join(".mcp.json");
    assert_eq!(
        std::path::Path::new(manifest),
        expected.as_path(),
        "mcp_manifest must be the plugin-root path under this data dir"
    );
    assert!(!std::path::Path::new(manifest).exists());
}

#[cfg(unix)]
#[test]
fn project_skill_lifecycle_uses_real_repository_paths() {
    Command::new("bash")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/test-project-skills.sh"
        ))
        .assert()
        .success();
}

#[cfg(unix)]
#[test]
fn badge_hook_without_home_or_host_configuration_is_nonfatal() {
    let binary = assert_cmd::cargo::cargo_bin!("comemory");
    let mut paths = vec![binary.parent().unwrap().to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    Command::new("bash")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/integrations/agent/hooks/comemory-status.sh"
        ))
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env_remove("HOME")
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("TOOLU_CONFIG_DIR")
        .env_remove("TOOLU_HOST_OVERRIDE")
        .env_remove("PLUGIN_ROOT")
        .write_stdin("{}")
        .assert()
        .success()
        .stdout("")
        .stderr("");
}
