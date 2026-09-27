#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
#![cfg(unix)]
//! `install.sh` finishes with a verified coordinator on the new binary, or
//! restores the previous binary and fails with exit 69 (#258 AC-1, AC-4,
//! AC-5). Real script under `sh`, real archives of the real binary from a
//! loopback release server, real processes; readiness faults are real
//! filesystem permissions or the real `external` supervisor.

#[path = "common/daemon_support.rs"]
mod daemon_support;
#[path = "common/install_rig.rs"]
mod install_rig;
#[path = "common/release_server.rs"]
mod release_server;

use std::path::Path;
use std::time::Duration;

use comemory::domains::sync::daemon::client::Probe;
use install_rig::{Rig, current_tag, file_id, parts, sha256_of};

const READY: Duration = Duration::from_secs(30);

fn is_root() -> bool {
    let out = std::process::Command::new("id").arg("-u").output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim() == "0"
}

/// A fault that stops every binary from reaching readiness: a data
/// directory the coordinator cannot bind in, or — where root ignores mode
/// bits — the `external` supervisor with nobody running `daemon run`.
fn blocking_fault(rig: &Rig) -> Vec<(&'static str, &'static str)> {
    if is_root() {
        return vec![("COMEMORY_DAEMON_SUPERVISOR", "external")];
    }
    set_mode(&rig.home.data_dir(), 0o500);
    Vec::new()
}

fn set_mode(path: &Path, mode: u32) {
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, mode);
    std::fs::set_permissions(path, perms).unwrap();
}

#[test]
fn a_fresh_logged_out_install_leaves_a_verified_coordinator_on_the_new_file() {
    let rig = Rig::new(&[]);
    let dir = rig.dir("bin");
    let stdout = rig.install_ok(&dir, &[]);

    let bin = std::fs::canonicalize(dir.join("comemory")).unwrap();
    let ready = rig.home.wait_ready(READY);
    assert_eq!(ready.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(ready.binary, bin);
    assert_eq!(ready.binary_file.as_deref(), Some(file_id(&bin).as_str()));
    assert_eq!(
        serde_json::to_value(ready.auth.state).unwrap(),
        "logged_out"
    );
    assert!(
        stdout.contains(&format!(
            "Sync daemon  ready ({}, pid {}",
            ready.version, ready.pid
        )),
        "{stdout}"
    );
    assert!(stdout.contains(&format!("comemory {} installed", current_tag())));
}

#[test]
fn quiet_and_no_completions_still_require_the_daemon() {
    let rig = Rig::new(&[]);
    let dir = rig.dir("bin");
    let stdout = rig.install_ok(&dir, &["--quiet", "--no-completions"]);
    assert!(stdout.is_empty(), "--quiet prints nothing: {stdout}");
    let ready = rig.home.wait_ready(READY);
    assert_eq!(
        ready.binary,
        std::fs::canonicalize(dir.join("comemory")).unwrap()
    );
}

#[test]
fn a_release_that_cannot_become_ready_is_rolled_back_and_the_coordinator_kept() {
    let rig = Rig::new(&["v9.9.9"]);
    let dir = rig.dir("bin");
    rig.install_ok(&dir, &[]);
    let before = rig.home.wait_ready(READY);
    let sha = sha256_of(&dir.join("comemory"));

    let (code, stdout, stderr) = parts(&rig.install(&dir, &["--version", "v9.9.9"], &[]));

    assert_eq!(code, 69, "{stdout}\n{stderr}");
    assert_eq!(
        sha256_of(&dir.join("comemory")),
        sha,
        "previous file restored"
    );
    assert!(stderr.contains("rolled back to comemory"), "{stderr}");
    assert!(stderr.contains("(sync daemon: ready)"), "{stderr}");
    assert!(!stdout.contains("installed"), "{stdout}");
    let after = rig.home.wait_ready(READY);
    assert_eq!(
        (after.pid, after.instance),
        (before.pid, before.instance),
        "the running coordinator was never evicted"
    );
    assert!(std::fs::read_dir(&dir).unwrap().all(|e| {
        let name = e.unwrap().file_name();
        !name.to_string_lossy().starts_with(".comemory.")
    }));
}

#[test]
fn a_fault_that_blocks_every_binary_reports_the_restored_daemon_not_ready() {
    let rig = Rig::new(&[]);
    let dir = rig.dir("bin");
    rig.install_ok(&dir, &[]);
    let sha = sha256_of(&dir.join("comemory"));
    let env = blocking_fault(&rig);

    let (code, stdout, stderr) = parts(&rig.install(&dir, &["--version", &current_tag()], &env));
    set_mode(&rig.home.data_dir(), 0o700);

    assert_eq!(code, 69, "{stdout}\n{stderr}");
    assert_eq!(sha256_of(&dir.join("comemory")), sha);
    assert!(
        stderr.contains("sync daemon not ready after installing"),
        "{stderr}"
    );
    assert!(stderr.contains("(sync daemon: not ready)"), "{stderr}");
    assert!(!stdout.contains("installed"), "{stdout}");
}

#[test]
fn a_first_install_that_cannot_become_ready_keeps_the_file_and_names_the_fix() {
    let rig = Rig::new(&[]);
    let dir = rig.dir("bin");
    let env = blocking_fault(&rig);

    let (code, stdout, stderr) = parts(&rig.install(&dir, &[], &env));
    set_mode(&rig.home.data_dir(), 0o700);

    assert_eq!(code, 69, "{stdout}\n{stderr}");
    assert!(
        dir.join("comemory").exists(),
        "a first install keeps the binary"
    );
    let fix = format!("{} sync daemon ensure", dir.join("comemory").display());
    assert!(stderr.contains(&fix), "{stderr}");
    assert!(!stdout.contains("installed"), "{stdout}");
}

/// Stage `tag` as a copy of the real release, then damage it with `harm`.
fn damaged(rig: &Rig, tag: &str, harm: impl Fn(&Path, &Path)) {
    let target = release_server::host_target().unwrap();
    let dir = rig.releases.join(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let archive = format!("comemory-{target}.tar.xz");
    let good = rig.releases.join(current_tag());
    for file in [
        archive.clone(),
        format!("{archive}.sha256"),
        "install.sh".into(),
    ] {
        std::fs::copy(good.join(&file), dir.join(&file)).unwrap();
    }
    harm(&dir.join(&archive), &dir.join(format!("{archive}.sha256")));
}

#[test]
fn pre_swap_failures_leave_the_previous_file_and_coordinator_untouched() {
    let rig = Rig::new(&[]);
    let dir = rig.dir("bin");
    rig.install_ok(&dir, &[]);
    let before = rig.home.wait_ready(READY);
    let sha = sha256_of(&dir.join("comemory"));

    damaged(&rig, "v8.8.1", |archive, sidecar| {
        let bytes = std::fs::read(archive).unwrap();
        std::fs::write(archive, &bytes[..bytes.len() / 2]).unwrap();
        let hash: String = sha256_of(archive)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        std::fs::write(sidecar, format!("{hash} *archive\n")).unwrap();
    });
    damaged(&rig, "v8.8.2", |_, sidecar| {
        std::fs::write(sidecar, format!("{} *archive\n", "0".repeat(64))).unwrap();
    });
    for tag in ["v8.8.1", "v8.8.2", "v8.8.3"] {
        let (code, stdout, stderr) = parts(&rig.install(&dir, &["--version", tag], &[]));
        assert_eq!(code, 1, "{tag}: {stdout}\n{stderr}");
        assert_eq!(sha256_of(&dir.join("comemory")), sha, "{tag}");
    }
    let Probe::Healthy(after) = rig.home.probe() else {
        panic!("the previous coordinator stopped answering");
    };
    assert_eq!((after.pid, after.instance), (before.pid, before.instance));
}
