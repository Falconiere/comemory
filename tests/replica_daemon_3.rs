#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The required resident daemon (#257), part 3: the hook-fired
//! `sync --action auto` wakes the coordinator over its socket instead of
//! running its own pass (A-5), and an inline save that could not fully
//! drain the outbox wakes it too (AC-5, AC-8).

#[path = "common/daemon_support.rs"]
mod daemon_support;

use std::process::Command;
use std::time::Duration;

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
