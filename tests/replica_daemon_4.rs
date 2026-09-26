#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The required resident daemon (#257), part 4: lifecycle and introspection
//! commands run no preflight at all (A-7), a required-preflight command
//! against an empty data directory never creates the corpus (D9/A-7),
//! `auth login`/`auth logout` (S8) drive the same coordinator through
//! `reload` rather than the pre-#257 opt-in install, and `comemory watch`
//! (S9) attaches to it too.

#[path = "common/auth_fixture.rs"]
mod auth_fixture;
#[path = "common/daemon_support.rs"]
mod daemon_support;
#[path = "common/device_auth_server.rs"]
mod device_auth_server;
#[path = "common/release_server.rs"]
mod release_server;
#[path = "common/sync_platform_server.rs"]
mod sync_platform_server;

use std::time::{Duration, Instant};

use comemory::domains::sync::daemon::client;
use comemory::domains::sync::daemon::control::Op;
use comemory::domains::sync::daemon::readiness::AuthState;
use daemon_support::DaemonHome;
use device_auth_server::DeviceAuthServer;
use release_server::ReleaseServer;
use serde_json::Value;
use sync_platform_server::{SyncPlatformServer, SyncPlatformState};

/// No `daemon.lock`, `daemon.sock` or `daemon.json` exists for `home`.
fn no_coordinator_artifacts(home: &DaemonHome) {
    for name in ["daemon.lock", "daemon.sock", "daemon.json"] {
        assert!(
            !home.data_dir().join(name).exists(),
            "{name} must not exist"
        );
    }
    assert!(home.coordinator_pids().is_empty());
}

#[test]
fn every_exempt_command_starts_no_coordinator() {
    let home = DaemonHome::new();
    let (code, _, stderr) = home.run(&["completions", "zsh"]);
    assert_eq!(code, 0, "{stderr}");
    no_coordinator_artifacts(&home);
    home.json(&["doctor"]);
    no_coordinator_artifacts(&home);
    // Logged out, `auth status` exits nonzero (not authenticated) — the
    // point here is only that it never touches the daemon.
    home.run(&["auth", "status"]);
    no_coordinator_artifacts(&home);
    // Logged out, `--action status` also needs a credential to open its
    // session — again, only the daemon-touching question matters here.
    home.run(&["sync", "--action", "status"]);
    no_coordinator_artifacts(&home);
    home.json(&["sync", "daemon", "status"]);
    no_coordinator_artifacts(&home);

    // Only whether `upgrade --check` touches the daemon matters here, not
    // whether it finds an upgrade — no release is staged under this tag.
    let root = home.root().join("releases");
    std::fs::create_dir_all(&root).unwrap();
    let server = ReleaseServer::start(root, "0.0.0-nonexistent");
    home.run_with_env(
        &["--json", "upgrade", "--check"],
        &[("COMEMORY_RELEASES_URL", &server.base)],
    );
    no_coordinator_artifacts(&home);

    let mut serving = home.spawn_serve();
    std::thread::sleep(Duration::from_millis(300));
    no_coordinator_artifacts(&home);
    let _ = serving.kill();
    let _ = serving.wait();
}

#[test]
fn a_required_command_over_an_empty_directory_never_creates_the_corpus() {
    let home = DaemonHome::new();
    let mut mcp = home.spawn_mcp_read_only();
    let ready = home.wait_ready(Duration::from_secs(15));
    assert_eq!(
        ready.store,
        comemory::domains::sync::daemon::readiness::StoreState::Absent,
        "the coordinator never opens the corpus for an empty directory"
    );
    let _ = mcp.kill();
    let _ = mcp.wait();
}

/// AC-4: writing a credential and sending `reload` — what `auth login`
/// does internally — flips the coordinator's own `auth.state`, and a second
/// workspace written the same way replaces the first rather than merging
/// with it.
#[test]
fn auth_lifecycle_reload_flips_auth_state_and_the_next_workspace_replaces_the_last() {
    let home = DaemonHome::new();
    home.json(&["sync", "daemon", "ensure"]);
    let started = home.wait_ready(Duration::from_secs(15));
    assert_eq!(started.auth.state, AuthState::LoggedOut, "{started:?}");

    auth_fixture::seed_org_auth(
        &home.paths(),
        "http://127.0.0.1:9/api",
        "cmk_first",
        "ws_first",
    );
    client::call::<Value>(&home.paths(), &Op::Reload {}, Duration::from_secs(2)).unwrap();
    let first = home.wait_for(Duration::from_secs(5), |r| {
        r.auth.state == AuthState::Authenticated
    });
    assert_eq!(first.auth.workspace_id.as_deref(), Some("ws_first"));
    assert_eq!(
        first.instance, started.instance,
        "reload updates the running coordinator; it never starts a new one"
    );

    auth_fixture::seed_org_auth(
        &home.paths(),
        "http://127.0.0.1:9/api",
        "cmk_second",
        "ws_second",
    );
    client::call::<Value>(&home.paths(), &Op::Reload {}, Duration::from_secs(2)).unwrap();
    // `AuthView` names exactly one workspace, so seeing it flip from
    // `ws_first` to `ws_second` (not, say, still `ws_first`, and not an
    // error) is what "replaces, never merges" looks like from the outside.
    let second = home.wait_for(Duration::from_secs(5), |r| {
        r.auth.workspace_id.as_deref() == Some("ws_second")
    });
    assert_ne!(
        second.auth.workspace_id, first.auth.workspace_id,
        "{second:?}"
    );
}

/// AC-4/D11: the logout barrier flips the coordinator to
/// `logged_out_barrier`, blocks a local `auth status` even with an
/// inherited `COMEMORY_API_KEY`, and never stops the coordinator itself.
#[test]
fn logout_barrier_blocks_every_credential_read_and_leaves_the_coordinator_running() {
    let home = DaemonHome::new();
    home.json(&["sync", "daemon", "ensure"]);
    let started = home.wait_ready(Duration::from_secs(15));
    auth_fixture::seed_org_auth(&home.paths(), "http://127.0.0.1:9/api", "cmk_x", "ws_x");

    std::fs::write(home.data_dir().join("config.toml"), "[broken").unwrap();
    let logout = home.json(&["auth", "logout"]);
    assert_eq!(logout["logged_out"], true, "{logout}");
    assert_eq!(
        logout["daemon"]["running"], true,
        "logout never stops the coordinator (D11): {logout}"
    );
    assert!(home.data_dir().join("auth.disabled").exists());
    assert!(!home.data_dir().join("auth.json").exists());

    // No credential read succeeds while the barrier stands, even one an
    // inherited COMEMORY_API_KEY would otherwise satisfy.
    let (code, stdout, _) = home.run_with_env(
        &["--json", "auth", "status"],
        &[("COMEMORY_API_KEY", "cmk_env_leak")],
    );
    assert_ne!(code, 0);
    let body: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(body["authenticated"], false, "{body}");

    let after = home.wait_for(Duration::from_secs(5), |r| {
        r.auth.state == AuthState::LoggedOutBarrier
    });
    assert_eq!(
        after.instance, started.instance,
        "the same instance is running afterwards (D11)"
    );
}

/// AC-8/compat: `--daemon` is a parseable, deprecated no-op — the
/// coordinator preflight already ensured is what answers, not an install
/// this flag triggers — and a real device login still reaches it through
/// `reload` with no second command.
#[test]
fn compat_the_deprecated_daemon_flag_changes_nothing_and_login_still_reaches_the_coordinator() {
    if !device_auth_server::tooling_present() {
        return;
    }
    let home = DaemonHome::new();
    let srv = DeviceAuthServer::start_default();

    let (code, stdout, stderr) = home.run(&[
        "--json",
        "auth",
        "login",
        "--daemon",
        "--api-url",
        &srv.base,
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stderr.contains("deprecated"),
        "--daemon must warn it is a no-op: stderr={stderr}"
    );
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["daemon"]["skipped"], false, "{report}");
    assert_eq!(
        report["daemon"]["running"], true,
        "preflight already ensured a coordinator for this Required command: {report}"
    );
    assert!(report["daemon"]["instance"].as_str().is_some(), "{report}");

    let status = home.json(&["sync", "daemon", "status"]);
    assert_eq!(
        status["daemon"]["auth"]["state"], "authenticated",
        "the coordinator picked up the credential without a second command: {status}"
    );
}

/// A memory only the fixture's org holds, queued for `GET /v1/sync/changes`
/// — the same fixture shape `cli__watch.rs` uses, seeded *after* login so
/// `catch_up` is what pulls it, not `auth login`'s own initial sync.
fn queue_a_change(srv: &SyncPlatformServer, body: &str) -> String {
    use sha2::{Digest as _, Sha256};
    let id = comemory::memory::id::memory_id(body);
    let content_hash = {
        let mut hasher = Sha256::new();
        hasher.update(body.trim_end().as_bytes());
        hasher.finalize().iter().fold(String::new(), |mut acc, b| {
            use std::fmt::Write as _;
            let _ = write!(acc, "{b:02x}");
            acc
        })
    };
    srv.update(|st| {
        st.consume_changes = true;
        st.head_seq += 1;
        st.changes = serde_json::json!([{
            "seq": st.head_seq,
            "op": "upsert",
            "id": id,
            "content_hash": content_hash,
            "at": "2026-09-26T00:00:00Z",
            "author": "someone-else",
            "record": {
                "frontmatter": {
                    "id": id,
                    "kind": "decision",
                    "repo": "",
                    "created": "2026-09-26T00:00:00Z",
                    "tags": [],
                    "quality": 3,
                    "schema": 1,
                    "content_hash": content_hash,
                },
                "body": body,
            }
        }]);
    });
    id
}

/// AC-9: `watch --once --json` attached to the coordinator prints
/// `connected` then `pulled`, and the announced memory exists locally
/// afterward. The workspace channel this coordinator also runs greets with
/// its own `hello` nudge on every (re)connect (A-5), which can itself pull
/// the change before `catch_up` gets to it — a real, desirable race (a
/// nudge should not wait on `watch`), not a bug this test fights: the
/// durable, deterministic claim is the file landing, not which of the two
/// mechanisms happened to move it this run.
#[test]
fn watch_once_attached_to_the_coordinator_pulls_a_memory_the_client_lacks() {
    if !device_auth_server::tooling_present() {
        return;
    }
    let srv = SyncPlatformServer::start(SyncPlatformState::default());
    let home = DaemonHome::new();
    // Long past this test's lifetime: cuts noise from the plain 5s
    // reconciliation tick, leaving only the channel's own nudge (unavoidable
    // — see above) as a second possible source of the pull.
    std::fs::write(
        home.data_dir().join("config.toml"),
        "[sync]\ndaemon_interval = \"1h\"\n",
    )
    .expect("config.toml");
    let (code, _, stderr) = home.run(&["auth", "login", "--api-url", &srv.base]);
    assert_eq!(code, 0, "{stderr}");
    home.wait_ready(Duration::from_secs(15));

    // Queued after login's own initial sync (and reload pass) drained
    // anything that was there before.
    let id = queue_a_change(&srv, "a decision pushed from the other laptop");

    let (code, stdout, stderr) = home.run(&["--json", "watch", "--once"]);
    assert_eq!(code, 0, "{stderr}");
    let events: Vec<Value> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("json line"))
        .collect();
    assert_eq!(
        events.first().and_then(|e| e["event"].as_str()),
        Some("connected"),
        "got {events:?}"
    );
    assert_eq!(
        events.get(1).and_then(|e| e["event"].as_str()),
        Some("pulled"),
        "got {events:?}"
    );

    let memories = std::fs::read_dir(home.data_dir().join("memories"))
        .expect("memories dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        memories
            .iter()
            .any(|name| name.starts_with(&format!("{id}-"))),
        "the announced memory must be on disk after watch --once: {memories:?}"
    );
}

/// AC-9: a manual `comemory sync` concurrent with the coordinator's own
/// passes leaves every local write forwarded exactly once — the outbox
/// empties and the pushed cursor lands exactly on the local log's head,
/// never ahead (a duplicate accepted twice) or short of it (one dropped).
#[test]
fn manual_sync_concurrent_with_coordinator_passes_drains_the_outbox_with_no_duplicate_or_lost_send()
{
    if !device_auth_server::tooling_present() {
        return;
    }
    let srv = SyncPlatformServer::start(SyncPlatformState::default());
    let home = DaemonHome::new();
    // Every save stays local until a manual `sync` or the coordinator's own
    // tick forwards it — otherwise push-on-save would drain the outbox
    // before the race below has anything to race over.
    std::fs::write(
        home.data_dir().join("config.toml"),
        "[sync]\npush_on_save = false\n",
    )
    .expect("config.toml");
    let (code, _, stderr) = home.run(&["auth", "login", "--api-url", &srv.base]);
    assert_eq!(code, 0, "{stderr}");
    let started = home.wait_ready(Duration::from_secs(15));
    assert_eq!(started.sync.interval_secs, 5);

    for i in 0..10 {
        let (code, _, stderr) = home.run(&[
            "save",
            "--repo",
            "acme/backend",
            &format!("concurrent write {i}"),
        ]);
        assert_eq!(code, 0, "{stderr}");
    }

    // Several manual `sync` calls race the coordinator's own interval tick,
    // already running against the same database and the same upstream.
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                let (code, _, stderr) = home.run(&["sync"]);
                assert!(code == 0 || code == 69, "sync errored ({code}): {stderr}");
            });
        }
    });

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let status = home.json(&["sync", "--action", "status"]);
        if status["pending"].as_i64() == Some(0) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the outbox never drained: {status}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let status = home.json(&["sync", "--action", "status"]);
    assert_eq!(status["pending"], 0, "{status}");
    assert_eq!(
        status["pushed_seq"], status["head_seq"],
        "every local write reached the upstream exactly once, none twice: {status}"
    );
}

#[test]
fn logout_removes_credentials_with_broken_config_and_failed_preflight() {
    let home = DaemonHome::new().env("COMEMORY_DAEMON_SUPERVISOR", "invalid");
    auth_fixture::seed_org_auth(&home.paths(), "http://127.0.0.1:9/api", "cmk_x", "ws_x");
    std::fs::write(home.data_dir().join("config.toml"), "[broken").unwrap();

    let logout = home.json(&["auth", "logout"]);
    assert_eq!(logout["logged_out"], true);
    assert!(home.data_dir().join("auth.disabled").exists());
    assert!(!home.data_dir().join("auth.json").exists());
    assert!(home.coordinator_pids().is_empty());
}
