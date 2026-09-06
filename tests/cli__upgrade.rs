#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
#![cfg(unix)]
//! `comemory upgrade` end to end: the real binary, a loopback stand-in for
//! GitHub Releases (`tests/common/release_server.rs`), the real `install.sh`,
//! real tarballs and checksums — and, where a swap happens, a PRIVATE copy of
//! the binary under `<tmp>/bin/`, because `upgrade` replaces whatever
//! `current_exe` is and that must never be `target/debug/comemory`, which
//! every other test in the run is executing.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use assert_cmd::cargo::cargo_bin;
use release_server::{ReleaseServer, host_target, stage_release, tooling_present};
use serde_json::Value;
use tempfile::TempDir;

#[path = "common/release_server.rs"]
mod release_server;

const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// A fixture root + server whose `/latest` names `latest`.
fn server(home: &TempDir, latest: &str) -> ReleaseServer {
    let root = home.path().join("releases");
    std::fs::create_dir_all(&root).unwrap();
    ReleaseServer::start(root, latest)
}

/// Copy the real binary to `<home>/bin/comemory` and return that path.
fn private_copy(home: &TempDir) -> PathBuf {
    let bin = home.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let dst = bin.join("comemory");
    std::fs::copy(cargo_bin("comemory"), &dst).unwrap();
    dst
}

fn run(exe: &Path, base: &str, home: &TempDir, args: &[&str]) -> Output {
    Command::new(exe)
        .env("COMEMORY_RELEASES_URL", base)
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .env("HOME", home.path())
        .args(args)
        .output()
        .expect("run comemory")
}

fn json(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "expected success, got {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn version_of(exe: &Path) -> String {
    let out = Command::new(exe).arg("--version").output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn check_reports_an_available_release_without_installing() {
    let home = TempDir::new().unwrap();
    let srv = server(&home, "v9.9.9");
    let exe = cargo_bin("comemory");
    let report = json(&run(
        &exe,
        &srv.base,
        &home,
        &["--json", "upgrade", "--check"],
    ));
    assert_eq!(report["status"], "available");
    assert_eq!(report["current"], CURRENT);
    assert_eq!(report["latest"], "9.9.9");
    assert_eq!(report["target"], "9.9.9");
    assert_eq!(report["channel"]["kind"], "standalone");
    assert_eq!(report["hint"], "run: comemory upgrade");
    assert_eq!(
        version_of(&exe),
        format!("comemory {CURRENT}"),
        "--check must not touch the binary"
    );
}

#[test]
fn check_reports_up_to_date_when_latest_is_the_running_build() {
    let home = TempDir::new().unwrap();
    let srv = server(&home, &format!("v{CURRENT}"));
    let exe = cargo_bin("comemory");
    let report = json(&run(
        &exe,
        &srv.base,
        &home,
        &["--json", "upgrade", "--check"],
    ));
    assert_eq!(report["status"], "up_to_date");
    assert_eq!(report["latest"], CURRENT);
    assert!(report.get("hint").is_none(), "no hint when nothing to do");
    let tty = run(&exe, &srv.base, &home, &["upgrade", "--check"]);
    assert!(tty.status.success());
    let stdout = String::from_utf8_lossy(&tty.stdout);
    assert!(stdout.contains("is up to date"), "{stdout}");
    assert!(stdout.contains("standalone"), "{stdout}");
}

#[test]
fn upgrade_replaces_the_running_binary_in_place() {
    if host_target().is_none() || !tooling_present() {
        return;
    }
    let home = TempDir::new().unwrap();
    let srv = server(&home, "v9.9.9");
    stage_release(
        &home.path().join("releases"),
        "v9.9.9",
        host_target().unwrap(),
    );
    let exe = private_copy(&home);
    let report = json(&run(&exe, &srv.base, &home, &["--json", "upgrade"]));
    assert_eq!(report["status"], "upgraded");
    assert_eq!(report["current"], CURRENT);
    assert_eq!(report["target"], "9.9.9");
    assert_eq!(
        report["exe"],
        Value::String(std::fs::canonicalize(&exe).unwrap().display().to_string())
    );
    assert_eq!(
        version_of(&exe),
        "comemory 9.9.9",
        "the file on disk is the release"
    );
    let leftovers: Vec<_> = std::fs::read_dir(exe.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n != "comemory")
        .collect();
    assert!(
        leftovers.is_empty(),
        "no staging files left behind: {leftovers:?}"
    );
}

#[test]
fn upgrade_is_a_no_op_when_already_on_latest() {
    let home = TempDir::new().unwrap();
    let srv = server(&home, &format!("v{CURRENT}"));
    let exe = private_copy(&home);
    let out = run(&exe, &srv.base, &home, &["upgrade"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The TTY line is colored (`0.18.2` is bold), so match its two halves.
    assert!(stdout.contains("is up to date"), "{stdout}");
    assert!(stdout.contains(CURRENT), "{stdout}");
    assert_eq!(version_of(&exe), format!("comemory {CURRENT}"));
}

#[test]
fn pinning_an_older_release_needs_force() {
    let home = TempDir::new().unwrap();
    let srv = server(&home, "v9.9.9");
    let exe = private_copy(&home);
    let out = run(&exe, &srv.base, &home, &["upgrade", "--version", "0.0.1"]);
    assert_eq!(out.status.code(), Some(64), "usage exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("0.0.1 is older than the running"),
        "{stderr}"
    );
    assert!(stderr.contains("--force"), "{stderr}");
    assert_eq!(
        version_of(&exe),
        format!("comemory {CURRENT}"),
        "nothing installed"
    );
}

#[test]
fn force_installs_a_pinned_older_release() {
    if host_target().is_none() || !tooling_present() {
        return;
    }
    let home = TempDir::new().unwrap();
    let srv = server(&home, "v9.9.9");
    stage_release(
        &home.path().join("releases"),
        "v0.0.1",
        host_target().unwrap(),
    );
    let exe = private_copy(&home);
    let out = run(
        &exe,
        &srv.base,
        &home,
        &["--json", "upgrade", "--version", "v0.0.1", "--force"],
    );
    let report = json(&out);
    assert_eq!(report["status"], "installed");
    assert_eq!(report["latest"], "9.9.9");
    assert_eq!(report["target"], "0.0.1");
    assert_eq!(version_of(&exe), "comemory 0.0.1");
}

#[test]
fn installer_failure_is_relayed_and_leaves_the_binary_alone() {
    if host_target().is_none() || !tooling_present() {
        return;
    }
    let home = TempDir::new().unwrap();
    let srv = server(&home, "v9.9.9");
    let root = home.path().join("releases");
    stage_release(&root, "v9.9.9", host_target().unwrap());
    let sidecar = root
        .join("v9.9.9")
        .join(format!("comemory-{}.tar.xz.sha256", host_target().unwrap()));
    std::fs::write(&sidecar, format!("{} *tampered\n", "0".repeat(64))).unwrap();
    let exe = private_copy(&home);
    let out = run(&exe, &srv.base, &home, &["--json", "upgrade"]);
    assert_eq!(out.status.code(), Some(70), "EX_SOFTWARE");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("install.sh exited with"), "{stderr}");
    assert!(
        stderr.contains("checksum mismatch"),
        "the script's own stderr is relayed: {stderr}"
    );
    assert_eq!(
        version_of(&exe),
        format!("comemory {CURRENT}"),
        "nothing installed"
    );
}

#[test]
fn unreachable_release_host_exits_unavailable() {
    let home = TempDir::new().unwrap();
    let exe = cargo_bin("comemory");
    let out = run(&exe, "http://127.0.0.1:1", &home, &["upgrade", "--check"]);
    assert_eq!(out.status.code(), Some(69), "EX_UNAVAILABLE");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("could not fetch http://127.0.0.1:1/latest"),
        "{stderr}"
    );
}

#[test]
fn a_malformed_pinned_version_is_a_usage_error() {
    let home = TempDir::new().unwrap();
    let srv = server(&home, "v9.9.9");
    let out = run(
        &cargo_bin("comemory"),
        &srv.base,
        &home,
        &["upgrade", "--version", "next"],
    );
    assert_eq!(out.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid version `next`"));
}
