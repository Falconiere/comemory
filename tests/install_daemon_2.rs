#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
#![cfg(unix)]
//! `install.sh`, part 2 (#258 AC-6, AC-7): racing installers and a
//! relocated install leave exactly one coordinator; the install lock
//! reclaims a dead owner and refuses an interrupted reclaim; an exported
//! `COMEMORY_API_KEY` never reaches the coordinator; uninstall removes only
//! this data directory's service.

#[path = "common/daemon_support.rs"]
mod daemon_support;
#[path = "common/install_rig.rs"]
mod install_rig;
#[path = "common/release_server.rs"]
mod release_server;

use std::path::Path;
use std::time::{Duration, Instant};

use comemory::domains::sync::daemon::client::Probe;
use daemon_support::{DaemonHome, alive};
use install_rig::{Rig, parts};

const READY: Duration = Duration::from_secs(30);

/// A pid that just exited (reaped), for a lock naming a dead owner.
fn dead_pid() -> u32 {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

fn wait_gone(pid: u32) {
    let deadline = Instant::now() + READY;
    while alive(pid) {
        assert!(Instant::now() < deadline, "pid {pid} still alive");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn racing_installers_into_one_directory_leave_one_coordinator() {
    let rig = Rig::new(&[]);
    let dir = rig.dir("bin");
    let outcomes: Vec<(i32, String, String)> = std::thread::scope(|s| {
        let a = s.spawn(|| parts(&rig.install(&dir, &[], &[])));
        let b = s.spawn(|| parts(&rig.install(&dir, &[], &[])));
        vec![a.join().unwrap(), b.join().unwrap()]
    });
    for (code, stdout, stderr) in &outcomes {
        assert!(
            *code == 0 || stderr.contains("another install into"),
            "{code}: {stdout}\n{stderr}"
        );
    }
    let ready = rig.home.wait_ready(READY);
    assert_eq!(
        ready.binary,
        std::fs::canonicalize(dir.join("comemory")).unwrap()
    );
    assert_eq!(rig.home.coordinator_pids(), vec![ready.pid]);
    assert!(
        !dir.join(".comemory-install.lock").exists(),
        "lock released"
    );
}

#[test]
fn relocating_the_install_replaces_the_coordinator_with_the_new_path() {
    let rig = Rig::new(&[]);
    rig.install_ok(&rig.dir("bin"), &[]);
    let old = rig.home.wait_ready(READY);

    let moved = rig.dir("elsewhere");
    rig.install_ok(&moved, &[]);

    let now = rig.home.wait_ready(READY);
    assert_eq!(
        now.binary,
        std::fs::canonicalize(moved.join("comemory")).unwrap()
    );
    assert_ne!(now.pid, old.pid);
    wait_gone(old.pid);
    assert_eq!(rig.home.coordinator_pids(), vec![now.pid]);
}

#[test]
fn a_lock_left_by_a_dead_installer_is_reclaimed() {
    let rig = Rig::new(&[]);
    let dir = rig.dir("bin");
    std::fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(dead_pid().to_string(), dir.join(".comemory-install.lock")).unwrap();

    rig.install_ok(&dir, &[]);
    assert!(!dir.join(".comemory-install.lock").exists());
    assert!(std::fs::symlink_metadata(dir.join(".comemory-install.lock")).is_err());
}

#[test]
fn an_interrupted_reclaim_stops_with_the_files_to_remove() {
    let rig = Rig::new(&[]);
    let dir = rig.dir("bin");
    std::fs::create_dir_all(&dir).unwrap();
    let owner = dead_pid();
    let lock = dir.join(".comemory-install.lock");
    std::os::unix::fs::symlink(owner.to_string(), &lock).unwrap();
    std::os::unix::fs::symlink(
        dead_pid().to_string(),
        dir.join(format!(".comemory-install.lock.reclaim.{owner}")),
    )
    .unwrap();

    let (code, stdout, stderr) = parts(&rig.install(&dir, &[], &[]));
    assert_eq!(code, 1, "{stdout}\n{stderr}");
    assert!(
        stderr.contains("interrupted while taking its lock"),
        "{stderr}"
    );
    assert!(stderr.contains(".comemory-install.lock*"), "{stderr}");
    assert!(!dir.join("comemory").exists(), "nothing was swapped");
}

/// The environment block of a live process, as text.
fn environment_of(pid: u32) -> String {
    if cfg!(target_os = "linux") {
        return String::from_utf8_lossy(&std::fs::read(format!("/proc/{pid}/environ")).unwrap())
            .replace('\0', "\n");
    }
    let out = std::process::Command::new("ps")
        .args(["-E", "-ww", "-o", "command=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Whether any regular file under `root` contains `needle`.
fn any_file_contains(root: &Path, needle: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|e| {
        let path = e.path();
        let meta = std::fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            return any_file_contains(&path, needle);
        }
        meta.is_file()
            && std::fs::read(&path).is_ok_and(|b| String::from_utf8_lossy(&b).contains(needle))
    })
}

#[test]
fn an_exported_api_key_never_reaches_the_coordinator_or_its_files() {
    let rig = Rig::new(&[]);
    let key = format!("cmk_{}{}", "installer", "shellsecret0123456789");
    let out = rig.install(&rig.dir("bin"), &[], &[("COMEMORY_API_KEY", key.as_str())]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let ready = rig.home.wait_ready(READY);
    let env = environment_of(ready.pid);
    assert!(
        env.contains("COMEMORY_DATA_DIR"),
        "environment was read: {env}"
    );
    assert!(!env.contains(&key), "key reached the coordinator: {env}");
    assert_eq!(
        serde_json::to_value(ready.auth.state).unwrap(),
        "logged_out"
    );
    assert!(!any_file_contains(&rig.home.data_dir(), &key));
    assert!(!any_file_contains(&rig.home.home_dir(), &key));
}

#[test]
fn uninstall_removes_only_this_directory_service_and_keeps_the_data() {
    let rig = Rig::new(&[]);
    let dir = rig.dir("bin");
    rig.install_ok(&dir, &[]);
    let bin = dir.join("comemory");
    let (code, _, stderr) = rig.home.run(&[
        "save",
        "--kind",
        "decision",
        "kept across uninstall: the corpus outlives the service",
    ]);
    assert_eq!(code, 0, "{stderr}");
    let other = DaemonHome::new();
    other.json(&["sync", "daemon", "ensure"]);
    let other_ready = other.wait_ready(READY);
    let ready = rig.home.wait_ready(READY);

    let out = rig
        .home
        .command_with_binary(&bin)
        .args(["--json", "sync", "daemon", "uninstall"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    wait_gone(ready.pid);
    assert!(
        !ready.socket.exists(),
        "socket removed: {}",
        ready.socket.display()
    );
    assert!(!rig.home.data_dir().join("daemon.json").exists());
    assert!(
        rig.home.data_dir().join("comemory.db").exists(),
        "the corpus stays"
    );
    let listed = rig
        .home
        .command_with_binary(&bin)
        .env("COMEMORY_SYNC_DAEMON", "0")
        .args(["--json", "list"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&listed.stdout).contains("kept across uninstall"));
    let Probe::Healthy(still) = other.probe() else {
        panic!("another data directory's coordinator was stopped");
    };
    assert_eq!(still.instance, other_ready.instance);
}
