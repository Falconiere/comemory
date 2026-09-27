#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
#![cfg(unix)]
//! `comemory upgrade` ends on a verified coordinator running the installed
//! binary (#258 AC-2, AC-3, AC-4(d), AC-9): the real binary installed by
//! the real `install.sh` into a private `bin/`, upgraded from there against
//! the loopback release server; pending operations cross the upgrade intact
//! and reach a real engine hub once it returns.

#[path = "common/daemon_hub_support.rs"]
mod daemon_hub_support;
#[path = "common/daemon_support.rs"]
mod daemon_support;
#[path = "common/exchange_support.rs"]
mod exchange_support;
#[path = "common/fault_proxy.rs"]
mod fault_proxy;
#[path = "common/install_rig.rs"]
mod install_rig;
#[path = "common/release_server.rs"]
mod release_server;
#[path = "common/replica_support.rs"]
mod replica_support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use daemon_hub_support::{seed_local_pending, write_auth};
use daemon_support::{DaemonHome, alive};
use exchange_support::Hub;
use install_rig::{Rig, current_tag, file_id, parts, sha256_of};
use serde_json::Value;

const READY: Duration = Duration::from_secs(30);

/// `<bin> --json <args>` in `home`'s environment against `rig`'s releases.
fn upgrade(rig: &Rig, home: &DaemonHome, bin: &Path, args: &[&str]) -> (i32, String, String) {
    let out = home
        .command_with_binary(bin)
        .env("COMEMORY_RELEASES_URL", &rig.srv.base)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("ZDOTDIR")
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    parts(&out)
}

fn upgrade_json(rig: &Rig, bin: &Path, args: &[&str]) -> Value {
    let (code, stdout, stderr) = upgrade(rig, &rig.home, bin, args);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    serde_json::from_str(&stdout).unwrap()
}

fn installed(rig: &Rig) -> PathBuf {
    let dir = rig.dir("bin");
    rig.install_ok(&dir, &["--quiet"]);
    std::fs::canonicalize(dir.join("comemory")).unwrap()
}

fn wait_gone(pid: u32) {
    let deadline = Instant::now() + READY;
    while alive(pid) {
        assert!(Instant::now() < deadline, "pid {pid} still alive");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_forced_reinstall_restarts_the_coordinator_on_the_new_file() {
    let rig = Rig::new(&[]);
    let bin = installed(&rig);
    let old = rig.home.wait_ready(READY);

    let report = upgrade_json(
        &rig,
        &bin,
        &["upgrade", "--force", "--version", &current_tag()],
    );

    assert_eq!(report["status"], "installed", "{report}");
    let daemon = &report["daemon"];
    assert_eq!(daemon["ready"], true, "{report}");
    assert_ne!(daemon["pid"].as_u64(), Some(u64::from(old.pid)), "{report}");
    assert_eq!(daemon["binary"], bin.display().to_string());
    assert_eq!(daemon["binary_file"], file_id(&bin));
    wait_gone(old.pid);
}

#[test]
fn an_up_to_date_upgrade_repairs_an_absent_coordinator() {
    let rig = Rig::new(&[]);
    let bin = installed(&rig);
    let old = rig.home.wait_ready(READY);
    rig.home.json(&["sync", "daemon", "stop"]);
    wait_gone(old.pid);

    let report = upgrade_json(&rig, &bin, &["upgrade"]);

    assert_eq!(report["status"], "up_to_date", "{report}");
    assert_eq!(report["daemon"]["ready"], true, "{report}");
    let now = rig.home.wait_ready(READY);
    assert_eq!(report["daemon"]["pid"].as_u64(), Some(u64::from(now.pid)));
}

#[test]
fn check_starts_nothing_and_writes_nothing() {
    let rig = Rig::new(&[]);
    let bin = installed(&rig);
    let fresh = DaemonHome::new();

    let (code, stdout, stderr) = upgrade(&rig, &fresh, &bin, &["upgrade", "--check"]);

    assert_eq!(code, 0, "{stdout}\n{stderr}");
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert!(report.get("daemon").is_none(), "{report}");
    for runtime in ["daemon.token", "daemon.json", "daemon.sock"] {
        assert!(
            !fresh.data_dir().join(runtime).exists(),
            "{runtime} written"
        );
    }
    assert!(fresh.coordinator_pids().is_empty());
}

#[test]
fn the_data_dir_flag_binds_the_coordinator_it_ensures() {
    let rig = Rig::new(&[]);
    let bin = installed(&rig);
    let other = DaemonHome::new();
    let target = other.data_dir();

    let report = upgrade_json(
        &rig,
        &bin,
        &[
            "--data-dir",
            target.to_str().unwrap(),
            "upgrade",
            "--force",
            "--version",
            &current_tag(),
        ],
    );

    assert_eq!(
        report["daemon"]["data_dir"],
        other.canonical().display().to_string()
    );
    assert_eq!(other.wait_ready(READY).binary, bin);
    assert!(
        !rig.home.home_dir().join(".comemory").exists(),
        "default dir untouched"
    );
}

#[test]
fn a_release_that_cannot_become_ready_fails_with_69_and_keeps_the_binary() {
    let rig = Rig::new(&["v9.9.9"]);
    let bin = installed(&rig);
    let sha = sha256_of(&bin);

    let (code, stdout, stderr) = upgrade(
        &rig,
        &rig.home,
        &bin,
        &["upgrade", "--force", "--version", "v9.9.9"],
    );

    assert_eq!(code, 69, "{stdout}\n{stderr}");
    assert!(stdout.trim().is_empty(), "no success JSON: {stdout}");
    assert!(stderr.contains("rolled back"), "{stderr}");
    assert_eq!(sha256_of(&bin), sha);
}

#[test]
fn a_cargo_install_build_is_pointed_at_the_finalizing_wrapper() {
    let rig = Rig::new(&["v9.9.9"]);
    // Canonical, as the running binary resolves its own path (macOS /tmp is
    // a symlink to /private/tmp).
    std::fs::create_dir_all(rig.home.root().join("cargo/bin")).unwrap();
    let cargo_home = std::fs::canonicalize(rig.home.root().join("cargo")).unwrap();
    let bin = cargo_home.join("bin/comemory");
    std::fs::copy(assert_cmd::cargo::cargo_bin("comemory"), &bin).unwrap();
    std::fs::write(
        cargo_home.join(".crates.toml"),
        format!(
            "[v1]\n\"comemory {} (path+file:///src/comemory)\" = [\"comemory\"]\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();

    let out = rig
        .home
        .command_with_binary(&bin)
        .env("CARGO_HOME", &cargo_home)
        .env("COMEMORY_RELEASES_URL", &rig.srv.base)
        .args(["upgrade", "--version", "v9.9.9"])
        .output()
        .unwrap();

    let (code, stdout, stderr) = parts(&out);
    assert_ne!(code, 0, "{stdout}");
    assert!(stderr.contains("bash scripts/dev-install.sh"), "{stderr}");
}

/// `operation_id`s of the client's journalled writes, oldest first.
fn outbox_ids(home: &DaemonHome) -> Vec<String> {
    let conn = rusqlite::Connection::open(home.paths().db_path()).unwrap();
    let mut statement = conn
        .prepare(
            "SELECT operation_id FROM replica_operation \
             WHERE entity_kind NOT IN ('feedback_event', 'activity_event') \
             AND state = 'pending' ORDER BY created_at, rowid",
        )
        .unwrap();
    statement
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap()
}

#[test]
fn pending_operations_cross_the_upgrade_and_reach_the_hub_once_it_returns() {
    let rig = Rig::new(&[]);
    let hub = Hub::start();
    std::fs::write(
        rig.home.data_dir().join("config.toml"),
        "[sync]\ndaemon_interval = \"1s\"\n",
    )
    .unwrap();
    write_auth(&rig.home, &hub);
    hub.proxy.set_upstream(None);
    seed_local_pending(&rig.home, &hub, 5, "acme/backend");
    let bin = installed(&rig);
    let old = rig.home.wait_ready(READY);
    let before = outbox_ids(&rig.home);
    assert_eq!(before.len(), 5, "five pending writes: {before:?}");

    let report = upgrade_json(
        &rig,
        &bin,
        &["upgrade", "--force", "--version", &current_tag()],
    );
    wait_gone(old.pid);

    assert_eq!(
        outbox_ids(&rig.home),
        before,
        "the outbox crossed the upgrade intact"
    );
    assert_eq!(
        report["daemon"]["data_dir"],
        rig.home.canonical().display().to_string()
    );
    hub.proxy
        .set_upstream(Some(exchange_support::addr_of(hub.engine())));
    let deadline = Instant::now() + Duration::from_secs(90);
    while !outbox_ids(&rig.home).is_empty() {
        assert!(Instant::now() < deadline, "the outbox never drained");
        std::thread::sleep(Duration::from_millis(300));
    }
    let feed: Vec<String> = hub.feed().into_iter().map(|(id, _, _)| id).collect();
    for id in &before {
        assert_eq!(
            feed.iter().filter(|f| *f == id).count(),
            1,
            "{id} reached the hub once"
        );
    }
    let now = rig.home.wait_ready(READY);
    assert_eq!(report["daemon"]["pid"].as_u64(), Some(u64::from(now.pid)));
}
