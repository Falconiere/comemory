#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The required resident daemon (#257), part 1: one verified coordinator per
//! canonical data directory (AC-1) and an idempotent, race-safe `ensure`
//! (AC-2). Real `comemory` processes, real Unix sockets, real signals.

#[path = "common/daemon_support.rs"]
mod daemon_support;

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::time::Duration;

use daemon_support::{DaemonHome, alive, signal};

use comemory::domains::sync::daemon::client::Probe;
use comemory::domains::sync::daemon::handshake;
use comemory::domains::sync::daemon::readiness::{AuthState, ChannelState, StoreState};

const READY: Duration = Duration::from_secs(30);

/// `auth.json` exactly as a login leaves it, naming nothing reachable.
fn write_auth(home: &DaemonHome, secret: &str) {
    let auth = serde_json::json!({
        "version": 2,
        "secret": secret,
        "key_prefix": "cmk_live",
        "api_url": "http://127.0.0.1:9/api",
        "organization_id": "org_daemon",
        "organization_slug": "daemon-org",
        "organization_name": "Daemon Org",
        "workspace_id": "ws_daemon",
    });
    std::fs::write(
        home.data_dir().join("auth.json"),
        serde_json::to_vec_pretty(&auth).unwrap(),
    )
    .unwrap();
}

#[test]
fn a_foreground_run_answers_with_verified_readiness() {
    let home = DaemonHome::new();
    let run = home.spawn_foreground(&[]);
    let r = home.wait_ready(READY);

    assert_eq!(r.protocol, 1);
    assert_eq!(r.version, env!("CARGO_PKG_VERSION"));
    let bin = std::fs::canonicalize(assert_cmd::cargo::cargo_bin("comemory")).unwrap();
    assert_eq!(r.binary, bin);
    assert_eq!(r.pid, run.pid());
    assert_eq!(r.instance.len(), 16);
    assert!(r.started_at.contains('T'));
    assert_eq!(r.data_dir, home.canonical());
    assert_eq!(r.socket, home.canonical().join("daemon.sock"));
    assert_eq!(r.supervisor, "foreground");
    assert_eq!(r.store, StoreState::Absent);
    assert_eq!(r.auth.state, AuthState::LoggedOut);
    assert_eq!(r.sync.interval_secs, 5);
    assert_eq!(r.sync.channel, ChannelState::Off);
    let mode = std::fs::metadata(&r.socket).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let record: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.data_dir().join("daemon.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["instance"], r.instance.as_str());
    assert!(
        !home.data_dir().join("comemory.db").exists(),
        "a coordinator never creates the corpus"
    );
}

#[test]
fn a_second_run_exits_75_while_the_first_keeps_answering() {
    let home = DaemonHome::new();
    let _first = home.spawn_foreground(&[]);
    let r = home.wait_ready(READY);
    let mut second = home.spawn_foreground(&[]);
    assert_eq!(
        second.wait_exit(Duration::from_secs(30)),
        Some(75),
        "{}",
        second.log()
    );
    assert!(second.log().contains("already running"), "{}", second.log());
    assert!(
        second.log().contains(&format!("pid {}", r.pid)),
        "{}",
        second.log()
    );
    let again = home.wait_ready(READY);
    assert_eq!(again.instance, r.instance, "the first one still answers");
}

#[test]
fn two_directories_get_two_coordinators_and_a_symlink_reaches_the_same_one() {
    let one = DaemonHome::new();
    let two = DaemonHome::new();
    let _a = one.spawn_foreground(&[]);
    let _b = two.spawn_foreground(&[]);
    let (ra, rb) = (one.wait_ready(READY), two.wait_ready(READY));
    assert_ne!(ra.pid, rb.pid);
    assert_ne!(ra.instance, rb.instance);

    let link = one.root().join("link");
    std::os::unix::fs::symlink(one.data_dir(), &link).unwrap();
    let through = comemory::domains::sync::daemon::client::probe(
        &comemory::config::paths::Paths::new(&link),
        Duration::from_secs(2),
    );
    let Probe::Healthy(via_link) = through else {
        panic!("{through:?}")
    };
    assert_eq!(via_link.instance, ra.instance);
    assert_eq!(via_link.data_dir, one.canonical());
}

#[test]
fn readiness_never_carries_the_secret_the_env_key_or_the_token() {
    let home = DaemonHome::new();
    let secret = "cmk_live_readiness_secret_value";
    let env_key = "cmk_env_override_readiness_value";
    write_auth(&home, secret);
    let run = home.spawn_foreground(&[("COMEMORY_API_KEY", env_key)]);
    let r = home.wait_for(READY, |r| r.auth.state == AuthState::Authenticated);
    assert_eq!(r.auth.workspace_id.as_deref(), Some("ws_daemon"));
    assert_eq!(r.auth.key_prefix.as_deref(), Some("cmk_live"));

    let token = handshake::load(&home.paths()).unwrap().expect("token");
    let answer = serde_json::to_string(&r).unwrap();
    let record = std::fs::read_to_string(home.data_dir().join("daemon.json")).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    for (name, text) in [
        ("readiness", answer),
        ("daemon.json", record),
        ("log", run.log()),
    ] {
        for needle in [secret, env_key, token.as_str()] {
            assert!(!text.contains(needle), "{name} leaks {needle}");
        }
    }
}

#[test]
fn a_client_with_a_wrong_proof_gets_nothing_and_is_hung_up_on() {
    let home = DaemonHome::new();
    let _run = home.spawn_foreground(&[]);
    let r = home.wait_ready(READY);
    let stream = std::os::unix::net::UnixStream::connect(&r.socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    writer
        .write_all(b"{\"hello\":1,\"nonce\":\"abc\"}\n")
        .unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(
        line.contains("\"proof\""),
        "the server proves itself first: {line}"
    );
    writer
        .write_all(
            format!(
                "{{\"proof\":\"{}\",\"op\":{{\"status\":{{}}}}}}\n",
                "0".repeat(64)
            )
            .as_bytes(),
        )
        .unwrap();
    line.clear();
    let read = reader.read_line(&mut line).unwrap();
    assert_eq!(read, 0, "no readiness for a wrong proof: {line}");
}

#[test]
fn a_leftover_socket_from_a_killed_run_is_bound_over() {
    let home = DaemonHome::new();
    let mut first = home.spawn_foreground(&[]);
    let r = home.wait_ready(READY);
    signal(first.pid(), "KILL");
    assert!(first.wait_exit(Duration::from_secs(30)).is_some());
    assert!(r.socket.exists(), "SIGKILL leaves the socket file behind");
    assert!(matches!(home.probe(), Probe::NotRunning(_)));

    let _second = home.spawn_foreground(&[]);
    let again = home.wait_ready(READY);
    assert_ne!(again.instance, r.instance);
    assert!(!alive(r.pid));
}

#[test]
fn sigterm_exits_zero_and_removes_the_socket_and_record() {
    let home = DaemonHome::new();
    let mut run = home.spawn_foreground(&[]);
    let r = home.wait_ready(READY);
    signal(run.pid(), "TERM");
    assert_eq!(
        run.wait_exit(Duration::from_secs(30)),
        Some(0),
        "{}",
        run.log()
    );
    assert!(!r.socket.exists(), "socket removed");
    assert!(
        !home.data_dir().join("daemon.json").exists(),
        "record removed"
    );
    assert!(matches!(home.probe(), Probe::NotRunning(_)));
}
