#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The required resident daemon (#257), part 3: the hook-fired
//! `sync --action auto` wakes the coordinator over its socket instead of
//! running its own pass (A-5), an inline save that could not fully drain
//! the outbox wakes it too (AC-5, AC-8), and its socket-path/lifecycle
//! failure modes reject-then-repair rather than silently misbehave (AC-6).

#[path = "common/daemon_support.rs"]
mod daemon_support;

use std::os::unix::fs::FileTypeExt as _;
use std::process::Command;
use std::time::{Duration, Instant};

use comemory::domains::sync::daemon::client::Probe;
use comemory::domains::sync::daemon::runtime_record::{self, RuntimeRecord};
use daemon_support::DaemonHome;

const READY: Duration = Duration::from_secs(30);

/// A real git repo named `repo` (the hook's `--repo` label, the checkout
/// directory's basename) with the real reindex hook installed.
fn hooked_repo(home: &DaemonHome) -> std::path::PathBuf {
    let repo = home.root().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    let status = Command::new("git")
        .arg("init")
        .arg(&repo)
        .env("PATH", home.hook_path())
        .status()
        .unwrap();
    assert!(status.success());
    for (key, value) in [("user.email", "test@example.com"), ("user.name", "Test")] {
        Command::new("git")
            .args(["config", key, value])
            .current_dir(&repo)
            .status()
            .unwrap();
    }
    let (code, stdout, stderr) = home.run(&["install-hooks", "--repo", repo.to_str().unwrap()]);
    assert_eq!(code, 0, "{stdout} {stderr}");
    repo
}

/// A real commit through the real `git`, so the real hook fires with
/// `PATH` naming only this build's `comemory` — never a host install.
fn commit(home: &DaemonHome, repo: &std::path::Path, file: &str, body: &str, message: &str) {
    std::fs::write(repo.join(file), body).unwrap();
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(repo)
        .status()
        .unwrap();
    let status = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(repo)
        .env("PATH", home.hook_path())
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn a_real_commits_hook_wakes_the_coordinator_which_indexes_the_checkout() {
    let home = DaemonHome::new();
    let repo = hooked_repo(&home);
    commit(&home, &repo, "src/a.ts", "export const a = 1;\n", "first");

    let ready = home.wait_ready(READY);
    assert_eq!(ready.sync.interval_secs, 5);
    // The hook backgrounds `sync --action auto` and returns immediately, so
    // the wake may still be in flight; wait for one pass to apply it.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let repos = home.json(&["repos"]);
        let rows = repos["repos"].as_array().cloned().unwrap_or_default();
        if rows.iter().any(|r| r["repo"] == "repo") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the checkout was never indexed: {repos}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn a_save_that_cannot_drain_the_outbox_wakes_the_coordinator() {
    let home = DaemonHome::new();
    // A credential naming an unreachable port: the inline push fails fast,
    // and `save` must still succeed and wake the daemon for it.
    home.json(&["sync", "daemon", "ensure"]);
    home.wait_ready(READY);
    let auth = serde_json::json!({
        "version": 2, "secret": "cmk_unreachable", "key_prefix": "cmk_test",
        "api_url": "http://127.0.0.1:9/api", "organization_id": "org",
        "organization_slug": "org", "organization_name": "Org", "workspace_id": "ws",
    });
    std::fs::write(
        home.data_dir().join("auth.json"),
        serde_json::to_vec_pretty(&auth).unwrap(),
    )
    .unwrap();

    // Captured before the save, so the save's own wake is what the poll
    // below observes — not a pass that happened to run first.
    let before = home.json(&["sync", "daemon", "status"])["daemon"]["sync"]["passes"]
        .as_u64()
        .unwrap();

    let saved = home.json(&["save", "--repo", "acme/backend", "owed while offline"]);
    assert!(saved["id"].as_str().is_some(), "{saved}");

    // The coordinator's own tick would also eventually pick this up
    // (interval_secs=5); the wake is what makes it show up sooner.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let status = home.json(&["sync", "daemon", "status"]);
        let passes = status["daemon"]["sync"]["passes"].as_u64();
        if passes.is_some_and(|p| p > before) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the coordinator never ran a pass within 3s of the wake: {status}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// AC-6: a plain file (not a socket) sitting at the socket path is not a
/// coordinator to negotiate with — `ensure` replaces it outright.
#[test]
fn a_stale_regular_file_at_the_socket_path_is_rejected_then_repaired() {
    let home = DaemonHome::new();
    std::fs::write(home.data_dir().join("daemon.sock"), b"not a socket").unwrap();
    let ensured = home.json(&["sync", "daemon", "ensure"]);
    assert_eq!(ensured["ready"], true, "{ensured}");
    home.wait_ready(READY);
    let meta = std::fs::symlink_metadata(home.data_dir().join("daemon.sock")).unwrap();
    assert!(
        meta.file_type().is_socket(),
        "the stale file must be replaced with a real socket"
    );
}

/// AC-6: a socket path that is a symlink into a *different* directory's real
/// coordinator must not be mistaken for this directory's own — the client
/// still holds its own directory's token, which fails that coordinator's
/// proof. `ensure` repairs with a coordinator of its own, replacing the
/// symlink.
#[test]
fn a_symlink_into_a_different_directory_coordinator_is_rejected_and_repaired() {
    let a = DaemonHome::new();
    let b = DaemonHome::new();
    a.json(&["sync", "daemon", "ensure"]);
    a.wait_ready(READY);
    b.json(&["sync", "daemon", "ensure"]);
    b.wait_ready(READY);
    let stop = b.json(&["sync", "daemon", "stop"]);
    assert_eq!(stop["stopped"], true, "{stop}");

    let sock = b.data_dir().join("daemon.sock");
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::fs::symlink_metadata(&sock).is_ok() && Instant::now() < deadline {
        let _ = std::fs::remove_file(&sock);
        std::thread::sleep(Duration::from_millis(50));
    }
    std::os::unix::fs::symlink(a.canonical().join("daemon.sock"), &sock).unwrap();

    let probe = b.probe();
    assert!(
        !matches!(probe, Probe::Healthy(_)),
        "b's own (stale) token must not authenticate against a's coordinator: {probe:?}"
    );
    let ensured = b.json(&["sync", "daemon", "ensure"]);
    assert_eq!(ensured["ready"], true, "{ensured}");
    let ready = b.wait_ready(READY);
    assert_eq!(ready.data_dir, b.canonical());
    let meta = std::fs::symlink_metadata(&sock).unwrap();
    assert!(
        !meta.file_type().is_symlink(),
        "the symlink must be replaced"
    );
}

/// AC-6: `daemon.json` naming a still-live process that is not `comemory`
/// (a stale record recycled onto an unrelated pid, say) must not be killed
/// by repair's courtesy cleanup — only a pid whose own reported command is
/// literally `comemory` is ever signaled.
#[test]
fn a_daemon_json_naming_a_live_non_comemory_pid_survives_repair() {
    let home = DaemonHome::new();
    let mut bystander = Command::new("sleep").arg("300").spawn().unwrap();
    let bystander_pid = bystander.id();
    let fake = RuntimeRecord {
        pid: bystander_pid,
        instance: "stale-instance".into(),
        socket: home.data_dir().join("daemon.sock"),
        version: "0.0.0".into(),
        binary: std::path::PathBuf::from("/nonexistent/comemory"),
        started_at: "2020-01-01T00:00:00Z".into(),
        supervisor: "process".into(),
    };
    runtime_record::write(&home.paths(), &fake).unwrap();

    let ensured = home.json(&["sync", "daemon", "ensure"]);
    assert_eq!(ensured["ready"], true, "{ensured}");
    assert!(
        daemon_support::alive(bystander_pid),
        "a live pid that is not comemory must survive repair"
    );
    let _ = bystander.kill();
    let _ = bystander.wait();
}

/// [`comemory::domains::sync::daemon::supervisor::signal_stale_pid`] itself:
/// it signals a pid only when that pid's own reported command is literally
/// `comemory` — checked directly here since nothing in the ordinary
/// `ensure` path can produce a *genuine* stale `comemory` process to prove
/// the positive case against.
#[test]
fn signal_stale_pid_ends_a_process_actually_named_comemory_but_spares_anything_else() {
    // Neither macOS nor Linux `ps -o comm=` reflects an `exec -a` argv[0]
    // rename — both report the executable's own file name. A real "process
    // actually named comemory" therefore needs a copy of the real binary
    // under that exact file name, running something that blocks.
    let root = tempfile::tempdir().unwrap();
    let copy = root.path().join("comemory");
    std::fs::copy(assert_cmd::cargo::cargo_bin("comemory"), &copy).unwrap();
    let mut perms = std::fs::metadata(&copy).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&copy, perms).unwrap();
    let mut impostor = Command::new(&copy)
        .args(["serve", "--port", "0"])
        .env("COMEMORY_DATA_DIR", root.path().join("data"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let impostor_pid = impostor.id();
    // Give it a moment to actually bind and settle into serving.
    std::thread::sleep(Duration::from_millis(300));
    comemory::domains::sync::daemon::supervisor::signal_stale_pid(impostor_pid).unwrap();
    // `impostor` is our own child: an unreaped zombie still answers `kill
    // -0` (what `daemon_support::alive` checks) as if it were running, so
    // this polls `try_wait` instead, which actually reaps it.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut exited = false;
    while Instant::now() < deadline {
        if impostor.try_wait().unwrap().is_some() {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !exited {
        // Clean up regardless of the assertion below, so a failure here
        // never leaves a real `comemory serve` running in the background.
        let _ = impostor.kill();
    }
    let _ = impostor.wait();
    assert!(exited, "a process actually named comemory must be signaled");

    let mut innocent = Command::new("sleep").arg("300").spawn().unwrap();
    let innocent_pid = innocent.id();
    comemory::domains::sync::daemon::supervisor::signal_stale_pid(innocent_pid).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        innocent.try_wait().unwrap().is_none(),
        "anything not named comemory must survive"
    );
    let _ = innocent.kill();
    let _ = innocent.wait();
}

/// AC-6: the socket file disappearing while the coordinator keeps running
/// (a stray `rm`, a tmpwatch) is noticed and re-bound within one guard tick
/// (1s) — the same instance, not a new coordinator.
#[test]
fn a_socket_unlinked_while_running_is_rebound_within_one_tick() {
    let home = DaemonHome::new();
    home.json(&["sync", "daemon", "ensure"]);
    let before = home.wait_ready(READY);
    std::fs::remove_file(home.data_dir().join("daemon.sock")).unwrap();

    let after = home.wait_for(Duration::from_secs(5), |r| r.instance == before.instance);
    assert_eq!(after.data_dir, home.canonical());
    let meta = std::fs::symlink_metadata(home.data_dir().join("daemon.sock")).unwrap();
    assert!(meta.file_type().is_socket());
}

/// AC-6: a changed `[sync] daemon_interval` is picked up by the running
/// coordinator's own next tick — no restart needed.
#[test]
fn a_changed_daemon_interval_is_reported_after_the_next_tick() {
    let home = DaemonHome::new();
    std::fs::write(
        home.data_dir().join("config.toml"),
        "[sync]\ndaemon_interval = \"1s\"\n",
    )
    .unwrap();
    home.json(&["sync", "daemon", "ensure"]);
    let started = home.wait_for(Duration::from_secs(5), |r| r.sync.interval_secs == 1);

    std::fs::write(
        home.data_dir().join("config.toml"),
        "[sync]\ndaemon_interval = \"2s\"\n",
    )
    .unwrap();
    let updated = home.wait_for(Duration::from_secs(5), |r| r.sync.interval_secs == 2);
    assert_eq!(
        updated.instance, started.instance,
        "the same coordinator picked up the change"
    );
}
