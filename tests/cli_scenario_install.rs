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
