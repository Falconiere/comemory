#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The required resident daemon, part 6 (#258): the coordinator's identity
//! includes the executable *file* it runs, so replacing the binary at the
//! same path, same version (a reinstall, a `cargo install --force` rebuild,
//! an installer killed right after its rename) restarts it on the new file.
//! The `process` spawn also never hands an inherited `COMEMORY_API_KEY` to
//! the resident coordinator.

#[path = "common/daemon_hub_support.rs"]
mod daemon_hub_support;
#[path = "common/daemon_support.rs"]
mod daemon_support;
#[path = "common/exchange_support.rs"]
mod exchange_support;
#[path = "common/fault_proxy.rs"]
mod fault_proxy;
#[path = "common/replica_support.rs"]
mod replica_support;

use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use daemon_hub_support::write_auth;
use daemon_support::{DaemonHome, alive};
use exchange_support::Hub;
use fault_proxy::Fault;

const READY: Duration = Duration::from_secs(30);

/// Put a fresh copy of the binary under test at `dest` the way an installer
/// does: write a staged file beside it, then rename it over `dest`. The
/// path stays the same; the inode changes.
fn install_copy(dest: &Path) -> PathBuf {
    let dir = dest.parent().expect("parent");
    std::fs::create_dir_all(dir).expect("bin dir");
    let staged = dir.join(format!(".comemory.new.{}", std::process::id()));
    std::fs::copy(assert_cmd::cargo::cargo_bin("comemory"), &staged).expect("copy binary");
    let mut perms = std::fs::metadata(&staged).expect("meta").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&staged, perms).expect("chmod +x");
    std::fs::rename(&staged, dest).expect("rename over");
    std::fs::canonicalize(dest).expect("canonical")
}

/// `<dev>:<ino>` of the file at `path` now.
fn file_id(path: &Path) -> String {
    let meta = std::fs::metadata(path).expect("stat");
    format!("{}:{}", meta.dev(), meta.ino())
}

/// `<bin> --json sync daemon ensure`, asserting exit 0.
fn ensure_from(home: &DaemonHome, bin: &Path, env: &[(&str, &str)]) -> serde_json::Value {
    let mut cmd = home.command_with_binary(bin);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .args(["--json", "sync", "daemon", "ensure"])
        .output()
        .expect("run ensure");
    assert!(
        out.status.success(),
        "ensure failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("ensure json")
}

fn pid_of(ensured: &serde_json::Value) -> u32 {
    u32::try_from(ensured["daemon"]["pid"].as_u64().expect("pid")).expect("pid fits")
}

fn wait_gone(pid: u32) {
    let deadline = Instant::now() + READY;
    while alive(pid) {
        assert!(Instant::now() < deadline, "pid {pid} still alive");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn ensure_replaces_a_same_version_coordinator_whose_file_was_replaced() {
    let home = DaemonHome::new();
    let bin = install_copy(&home.root().join("bin/comemory"));
    let before = ensure_from(&home, &bin, &[]);
    let old_pid = pid_of(&before);

    install_copy(&bin);
    let after = ensure_from(&home, &bin, &[]);

    assert_eq!(after["ready"], true, "{after}");
    assert_ne!(pid_of(&after), old_pid, "a new coordinator: {after}");
    assert_eq!(after["daemon"]["binary"], bin.display().to_string());
    assert_eq!(after["daemon"]["binary_file"], file_id(&bin), "{after}");
    wait_gone(old_pid);
}

#[test]
fn preflight_replaces_a_coordinator_left_running_after_a_killed_installer_renamed_the_file() {
    let home = DaemonHome::new();
    let bin = install_copy(&home.root().join("bin/comemory"));
    let old_pid = pid_of(&ensure_from(&home, &bin, &[]));

    // The installer died between its rename and its `ensure`.
    install_copy(&bin);
    let _ = home
        .command_with_binary(&bin)
        .args(["--json", "stats"])
        .output()
        .expect("run stats");

    let replaced = home.wait_for(READY, |r| r.pid != old_pid);
    assert_eq!(replaced.binary, bin);
    wait_gone(old_pid);
}

#[test]
fn preflight_keeps_the_coordinator_when_the_file_is_unchanged() {
    let home = DaemonHome::new();
    let bin = install_copy(&home.root().join("bin/comemory"));
    let first = ensure_from(&home, &bin, &[]);
    let _ = home
        .command_with_binary(&bin)
        .args(["--json", "stats"])
        .output()
        .expect("run stats");
    let after = home.wait_ready(READY);
    assert_eq!(after.pid, pid_of(&first), "no churn for an unchanged file");
}

/// The environment block of a live process, as text.
fn environment_of(pid: u32) -> String {
    if cfg!(target_os = "linux") {
        return String::from_utf8_lossy(
            &std::fs::read(format!("/proc/{pid}/environ")).expect("read environ"),
        )
        .replace('\0', "\n");
    }
    let out = std::process::Command::new("ps")
        .args(["-E", "-ww", "-o", "command=", "-p", &pid.to_string()])
        .output()
        .expect("ps -E");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_process_supervised_coordinator_never_inherits_the_api_key() {
    let home = DaemonHome::new();
    let key = format!("cmk_{}{}", "inherit", "edsecret0123456789");
    let ensured = ensure_from(
        &home,
        &assert_cmd::cargo::cargo_bin("comemory"),
        &[("COMEMORY_API_KEY", key.as_str())],
    );
    let pid = pid_of(&ensured);
    let env = environment_of(pid);
    assert!(
        env.contains("COMEMORY_DATA_DIR"),
        "the environment was read: {env}"
    );
    assert!(
        !env.contains(&key),
        "key leaked into the coordinator: {env}"
    );
    assert_eq!(
        serde_json::to_value(home.wait_ready(READY).auth.state).unwrap(),
        "logged_out"
    );
}

#[test]
fn a_descriptor_the_caller_leaked_never_pins_the_callers_pipe_open() {
    let home = DaemonHome::new();
    // The shell duplicates its stdout pipe onto fd 7 without CLOEXEC — the
    // same inheritance a multithreaded parent's racing pipe creation
    // produces. A coordinator that kept fd 7 would hold this pipe's write
    // end forever, so reading the command's output would never end.
    let mut cmd = home.command_with_binary(Path::new("sh"));
    cmd.args(["-c", "exec 7>&1; exec \"$0\" --json sync daemon ensure"])
        .arg(assert_cmd::cargo::cargo_bin("comemory"));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(cmd.output());
    });
    let out = rx
        .recv_timeout(READY)
        .expect("the caller's pipe reached EOF")
        .expect("run ensure");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    home.wait_ready(READY);
}

#[test]
fn a_coordinator_draining_a_held_pass_is_still_replaced_within_ensure() {
    let hub = Hub::start();
    let home = DaemonHome::new();
    drop(comemory::store::connection::open(home.paths().db_path()).unwrap());
    std::fs::write(
        home.data_dir().join("config.toml"),
        "[sync]\ndaemon_interval = \"1s\"\n",
    )
    .unwrap();
    write_auth(&home, &hub);
    let bin = install_copy(&home.root().join("bin/comemory"));
    let old_pid = pid_of(&ensure_from(&home, &bin, &[]));

    // The old coordinator's next tick blocks inside an upstream request, so
    // its graceful stop spends its whole grace waiting for that pass.
    hub.proxy.arm(Fault::HoldRequest {
        path: "/sync/replica/changes".into(),
    });
    std::thread::sleep(Duration::from_millis(2500));
    install_copy(&bin);
    let after = ensure_from(&home, &bin, &[]);
    hub.proxy.release();

    assert_eq!(after["ready"], true, "{after}");
    assert_ne!(pid_of(&after), old_pid, "{after}");
    assert_eq!(after["daemon"]["binary_file"], file_id(&bin), "{after}");
    wait_gone(old_pid);
}
