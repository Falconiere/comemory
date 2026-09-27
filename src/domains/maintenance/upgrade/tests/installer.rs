#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `verify_ensured` over the exact document `sync daemon ensure --json`
//! prints (`Ensured::to_json`): a ready coordinator must answer with the
//! installed version at the installed binary's canonical path. The
//! end-to-end run of the real child is in `tests/upgrade_daemon.rs`.

use std::path::Path;

use super::verify_ensured;
use crate::domains::maintenance::upgrade::version::Version;
use crate::domains::sync::daemon::ensure::Ensured;
use crate::domains::sync::daemon::readiness::Readiness;
use crate::domains::sync::daemon::supervisor::Kind;

fn readiness(binary: &Path, version: &str) -> Readiness {
    serde_json::from_value(serde_json::json!({
        "protocol": 1, "version": version, "binary": binary, "binary_file": "1:2",
        "pid": 4242, "instance": "i", "started_at": "2026-09-27T00:00:00Z",
        "data_dir": "/d", "socket": "/d/daemon.sock", "supervisor": "process",
        "store": "absent", "auth": {"state": "logged_out"},
        "sync": {"state": "idle", "interval_secs": 5, "passes": 0, "queued": false,
                 "channel": "off"}
    }))
    .unwrap()
}

fn ensured(daemon: Option<Readiness>, error: Option<&str>) -> serde_json::Value {
    Ensured {
        ready: daemon.is_some(),
        action: "replaced",
        supervisor: Kind::Process,
        notes: Vec::new(),
        daemon,
        error: error.map(str::to_string),
    }
    .to_json()
}

/// A real file, canonicalized the way the coordinator reports its binary.
fn real_binary(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("comemory");
    std::fs::write(&path, "bin").unwrap();
    std::fs::canonicalize(path).unwrap()
}

#[test]
fn a_ready_coordinator_on_the_installed_version_and_path_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let bin = real_binary(dir.path());
    let doc = ensured(Some(readiness(&bin, "0.50.1")), None);
    let report = verify_ensured(&doc, &bin, &Version::parse("0.50.1").unwrap()).unwrap();
    assert_eq!(report.pid, 4242);
    assert_eq!(report.binary, bin);
    assert_eq!(report.binary_file.as_deref(), Some("1:2"));
    assert_eq!(report.action, "replaced");
    assert_eq!(report.supervisor, "process");
}

#[test]
fn another_version_or_path_is_not_the_installed_binary() {
    let dir = tempfile::tempdir().unwrap();
    let bin = real_binary(dir.path());
    let want = Version::parse("0.50.1").unwrap();
    let old = verify_ensured(&ensured(Some(readiness(&bin, "0.50.0")), None), &bin, &want);
    assert_eq!(
        old.unwrap_err(),
        format!(
            "the coordinator answers as 0.50.0 at {0}, expected 0.50.1 at {0}",
            bin.display()
        )
    );
    let elsewhere = Path::new("/elsewhere/comemory");
    let moved = verify_ensured(
        &ensured(Some(readiness(elsewhere, "0.50.1")), None),
        &bin,
        &want,
    );
    assert_eq!(
        moved.unwrap_err(),
        format!(
            "the coordinator answers as 0.50.1 at /elsewhere/comemory, expected 0.50.1 at {}",
            bin.display()
        )
    );
}

#[test]
fn a_not_ready_document_carries_its_error() {
    let dir = tempfile::tempdir().unwrap();
    let bin = real_binary(dir.path());
    let doc = ensured(None, Some("sync daemon not ready (process)"));
    let err = verify_ensured(&doc, &bin, &Version::parse("0.50.1").unwrap()).unwrap_err();
    assert_eq!(err, "sync daemon not ready (process)");
}
