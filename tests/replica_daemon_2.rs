#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The required resident daemon (#257), part 2: CLI preflight repairs a
//! missing/stale coordinator before an ordinary command proceeds, or fails
//! it with an actionable local-service error (AC-3).

#[path = "common/daemon_support.rs"]
mod daemon_support;

use std::time::Duration;

use daemon_support::DaemonHome;
use nix::unistd::{Pid, getsid};

use comemory::domains::sync::daemon::client::Probe;

const READY: Duration = Duration::from_secs(30);

#[test]
fn uninstall_emits_one_json_document_and_stops_the_coordinator() {
    let home = DaemonHome::new();
    home.json(&["sync", "daemon", "ensure"]);
    home.wait_ready(READY);
    assert_eq!(
        home.json(&["sync", "daemon", "uninstall"]),
        serde_json::json!({"uninstalled": true})
    );
    let deadline = std::time::Instant::now() + READY;
    while !matches!(home.probe(), Probe::NotRunning(_)) {
        assert!(
            std::time::Instant::now() < deadline,
            "coordinator still running"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn process_supervised_coordinator_owns_a_detached_session() {
    let home = DaemonHome::new();
    home.json(&["sync", "daemon", "ensure"]);
    let ready = home.wait_ready(READY);
    let process = Pid::from_raw(ready.pid.try_into().unwrap());
    let session_id = getsid(Some(process)).unwrap();
    assert_eq!(
        session_id.as_raw(),
        process.as_raw(),
        "the process-supervised coordinator must own its terminal-detached session"
    );
}

#[test]
fn sixteen_parallel_commands_self_heal_a_sigkilled_coordinator_into_one() {
    let home = std::sync::Arc::new(DaemonHome::new());
    // Establish a coordinator, then kill it the way a crash would.
    let first = home.json(&["sync", "daemon", "ensure"]);
    let pid: u32 = first["daemon"]["pid"].as_u64().unwrap().try_into().unwrap();
    daemon_support::signal(pid, "KILL");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while daemon_support::alive(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!daemon_support::alive(pid), "the coordinator is dead");

    let handles: Vec<_> = (0..16)
        .map(|_| {
            let home = std::sync::Arc::clone(&home);
            std::thread::spawn(move || home.json(&["list"]))
        })
        .collect();
    for handle in handles {
        let out = handle.join().unwrap();
        assert!(out.is_object(), "{out}");
    }
    let healed = home.wait_ready(READY);
    assert_ne!(
        healed.pid, pid,
        "a fresh coordinator replaced the killed one"
    );
    assert_eq!(home.coordinator_pids().len(), 1, "no process storm");
}

#[test]
fn an_unreachable_external_supervisor_fails_the_command_with_a_named_fix() {
    let home = DaemonHome::new().env("COMEMORY_DAEMON_SUPERVISOR", "external");
    let (code, stdout, stderr) = home.run(&["list"]);
    assert_eq!(code, 69, "stdout={stdout} stderr={stderr}");
    assert!(
        stderr.contains("comemory sync daemon run"),
        "the fix names the foreground supervisor: {stderr}"
    );
    assert!(
        matches!(home.probe(), Probe::NotRunning(_)),
        "external supervision never self-spawns"
    );
}
