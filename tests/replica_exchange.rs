#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The exchange client (#255), part 1: negotiation over real engines — which
//! protocol a key selects, how an old server is covered, and that an auth or
//! protocol failure never downgrades (X-1, AC-1…AC-3) — plus the fault proxy's
//! own guarantees, which every later suite leans on.
//!
//! Real `comemory serve` hubs, the real CLI binary, real SQLite, real HTTP.

#[path = "common/exchange_support.rs"]
mod exchange_support;
#[path = "common/fault_proxy.rs"]
mod fault_proxy;
#[path = "common/replica_support.rs"]
mod replica_support;

use std::time::Duration;

use exchange_support::Hub;
use fault_proxy::{Fault, Outcome};

/// A GET through the proxy, the way a client makes one.
fn get_through(hub: &Hub, path: &str) -> (u16, String) {
    let response = reqwest::blocking::Client::new()
        .get(format!("{}{path}", hub.proxy.origin()))
        .bearer_auth(hub.token())
        .timeout(Duration::from_secs(10))
        .send()
        .expect("GET through the proxy");
    let status = response.status().as_u16();
    (status, response.text().unwrap_or_default())
}

#[test]
fn fault_proxy_forwards_the_engines_bytes_unchanged() {
    let hub = Hub::start();
    let (direct_status, direct) = hub.engine().get("/api/v1/sync/replica/manifest");
    let (status, through) = get_through(&hub, "/api/v1/sync/replica/manifest");
    let through: serde_json::Value = serde_json::from_str(&through).expect("json");
    assert_eq!(status, direct_status);
    assert_eq!(
        through["data"]["stream_epoch"], direct["data"]["stream_epoch"],
        "the proxied answer is the engine's own"
    );
    assert_eq!(
        hub.proxy.log()[0].outcome,
        Outcome::Forwarded(200),
        "and the log says it was relayed"
    );
}

#[test]
fn fault_proxy_refuses_above_the_rate_with_retry_after_and_never_forwards_it() {
    let hub = Hub::start();
    hub.proxy.arm(Fault::RateLimit {
        max: 1,
        retry_after: 2,
    });
    let (first, _) = get_through(&hub, "/api/v1/sync/replica/manifest");
    let response = reqwest::blocking::Client::new()
        .get(format!(
            "{}/api/v1/sync/replica/manifest",
            hub.proxy.origin()
        ))
        .bearer_auth(hub.token())
        .send()
        .expect("second GET");
    assert_eq!(first, 200);
    assert_eq!(response.status().as_u16(), 429);
    assert_eq!(
        response.headers()["retry-after"].to_str().expect("header"),
        "2"
    );
    assert_eq!(hub.proxy.log()[1].outcome, Outcome::RateLimited);
}

#[test]
fn fault_proxy_answers_bad_gateway_while_the_engine_is_down_and_recovers() {
    let mut hub = Hub::start();
    let old_token = hub.token();
    hub.stop();
    let (down, _) = get_through_with(&hub, "/api/v1/sync/replica/manifest", &old_token);
    assert_eq!(down, 502, "a stopped engine reads as a gateway error");
    hub.restart();
    let (up, _) = get_through(&hub, "/api/v1/sync/replica/manifest");
    assert_eq!(up, 200, "and the restarted engine answers again");
    let (revoked, _) = get_through_with(&hub, "/api/v1/sync/replica/manifest", &old_token);
    assert_eq!(revoked, 401, "the restart rotated the credential for real");
}

#[test]
fn fault_proxy_drops_a_response_after_the_engine_applied_the_request() {
    let hub = Hub::start();
    hub.proxy.arm(Fault::DropResponse {
        path: "/sync/replica/import".into(),
        times: 1,
    });
    let before = hub.feed().len();
    let body = serde_json::json!({
        "protocol": "replica-v1",
        "operations": [{
            "operation_id": "op-20260924-00000000000000000000000000000001",
            "entity_kind": "memory",
            "entity_key": "a1b2c3d4",
            "op": "tombstone",
            "schema_version": 1,
        }]
    });
    let sent = reqwest::blocking::Client::new()
        .post(format!("{}/api/v1/sync/replica/import", hub.proxy.origin()))
        .bearer_auth(hub.token())
        .json(&body)
        .send();
    assert!(sent.is_err(), "the client never sees the acknowledgement");
    assert_eq!(hub.feed().len(), before + 1, "but the engine accepted it");
    assert_eq!(hub.proxy.log()[0].outcome, Outcome::Dropped);
}

#[test]
fn fault_proxy_flips_one_byte_of_one_response_and_holds_until_released() {
    let hub = Hub::start();
    hub.proxy.arm(Fault::FlipResponse {
        path: "/sync/replica/manifest".into(),
        needle: "replica-v1".into(),
        replacement: "replica-v9".into(),
    });
    let (_, flipped) = get_through(&hub, "/api/v1/sync/replica/manifest");
    assert!(flipped.contains("replica-v9"));
    let (_, clean) = get_through(&hub, "/api/v1/sync/replica/manifest");
    assert!(
        clean.contains("\"replica-v1\""),
        "only one response is corrupted"
    );

    hub.proxy.arm(Fault::HoldRequest {
        path: "/sync/replica/changes".into(),
    });
    let origin = hub.proxy.origin();
    let token = hub.token();
    let waiter = std::thread::spawn(move || {
        reqwest::blocking::Client::new()
            .get(format!(
                "{origin}/api/v1/sync/replica/changes?since=0&limit=5"
            ))
            .bearer_auth(token)
            .send()
            .map(|r| r.status().as_u16())
    });
    assert!(
        hub.proxy.wait_held(Duration::from_secs(5)),
        "the request is parked"
    );
    assert!(!waiter.is_finished(), "and nothing answered it yet");
    hub.proxy.release();
    assert_eq!(waiter.join().expect("join").expect("answered"), 200);
}

/// Whether `pid` still names a process (a zombie included), asked of the
/// real `kill -0`.
fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[test]
fn a_spawned_daemon_is_reaped_when_its_guard_drops() {
    let hub = Hub::start();
    let client = exchange_support::Client::new();
    client.login(&hub);
    let daemon = client.spawn(&["sync", "daemon", "run"], &[]);
    let pid = daemon.id();
    assert!(alive(pid), "the daemon is running");

    // What a failed assertion or an early return does to the guard.
    drop(daemon);

    assert!(
        !alive(pid),
        "dropping the guard killed and reaped the daemon"
    );
}

fn get_through_with(hub: &Hub, path: &str, token: &str) -> (u16, String) {
    let response = reqwest::blocking::Client::new()
        .get(format!("{}{path}", hub.proxy.origin()))
        .bearer_auth(token)
        .timeout(Duration::from_secs(10))
        .send()
        .expect("GET through the proxy");
    let status = response.status().as_u16();
    (status, response.text().unwrap_or_default())
}

// ---------------------------------------------------------------------------
// X-1: negotiation (AC-1…AC-3)
// ---------------------------------------------------------------------------

use exchange_support::{Client, REPO, WORKSPACE, prejournal_memories};

/// A client logged into `hub` with `REPO` approved.
fn approved_client(hub: &Hub) -> Client {
    let client = Client::new();
    client.login(hub);
    client.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    client
}

/// Every outbox row of `client` in the given state.
fn in_state(client: &Client, state: &str) -> Vec<String> {
    client
        .outbox()
        .into_iter()
        .filter(|(_, s, _)| s == state)
        .map(|(id, _, _)| id)
        .collect()
}

#[test]
fn negotiation_selects_replica() {
    let hub = Hub::start();
    let (_, manifest) = hub.engine().get("/api/v1/sync/replica/manifest");
    let capabilities: Vec<String> =
        serde_json::from_value(manifest["data"]["capabilities"].clone()).expect("capabilities");
    for kind in [
        "replica-v1",
        "memory@1",
        "code_generation@1",
        "document_revision@1",
        "feedback_event@1",
        "activity_event@1",
    ] {
        assert!(
            capabilities.contains(&kind.to_string()),
            "the engine advertises {kind}: {capabilities:?}"
        );
    }

    let client = approved_client(&hub);
    client.save(
        "negotiation picks the journal protocol when the hub offers it",
        REPO,
    );
    client.sync();

    let status = client.exchange_status();
    assert_eq!(status["protocol"], "replica-v1", "{status}");
    assert_eq!(status["coverage"], "full");
    assert!(status["coverage_reason"].is_null());
    assert_eq!(status["api_url"], hub.api_url());
    assert_eq!(status["workspace"], WORKSPACE);
    let accepted = in_state(&client, "accepted");
    assert_eq!(
        accepted.len(),
        1,
        "the save went out: {:?}",
        client.outbox()
    );
    assert!(
        hub.feed().iter().any(|(op, _, _)| op == &accepted[0]),
        "under the id the client minted"
    );

    client.sync();
    assert_eq!(
        client.exchange_status()["protocol"],
        "replica-v1",
        "the selection is persisted for the key"
    );
}

#[test]
fn old_server_partial_then_upgrade_once() {
    // 250 memories the hub's journal has never seen: seeding runs 200 per
    // replica call, and a client run makes exactly one manifest call, so the
    // first run finds the hub still seeding.
    let hub = Hub::start_prepared(|data| prejournal_memories(data, 250, "acme/hub-only"));
    let client = approved_client(&hub);
    client.approve(&hub.api_url(), WORKSPACE, &[REPO, "acme/skipped"], 1);
    // No inline push after each save: every inline push opens a session,
    // and each session's manifest call would advance the hub's seeding.
    std::fs::write(
        client.data_dir().join("config.toml"),
        "[sync]\nskip_repos = [\"acme/skipped\"]\npush_on_save = false\n",
    )
    .expect("config");
    let first = client.save(
        "the old protocol carries this while the hub is still seeding",
        REPO,
    );
    let second = client.save("and this one, through the engine's own legacy routes", REPO);
    client.save(
        "skip_repos withholds this one on either protocol",
        "acme/skipped",
    );

    client.sync();
    let status = client.exchange_status();
    assert_eq!(status["protocol"], "legacy", "{status}");
    assert_eq!(status["coverage"], "partial");
    assert_eq!(status["coverage_reason"], "upstream_not_ready");
    let hub_ids: Vec<String> = {
        let conn = hub.db();
        let mut statement = conn
            .prepare("SELECT id FROM memories WHERE deleted_at IS NULL")
            .expect("prepare");
        statement
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect")
    };
    assert!(
        hub_ids.contains(&first) && hub_ids.contains(&second),
        "the legacy routes delivered both approved memories"
    );

    client.sync();
    let status = client.exchange_status();
    assert_eq!(
        status["protocol"], "replica-v1",
        "the finished hub is adopted: {status}"
    );
    assert_eq!(status["coverage"], "full");
    let feed = hub.feed();
    for delivered in [&first, &second] {
        assert_eq!(
            feed.iter().filter(|(_, key, _)| key == delivered).count(),
            1,
            "nothing the legacy push delivered was sent a second time: {delivered}"
        );
    }
    let rows = client.outbox();
    let settled = rows.iter().filter(|(_, s, _)| s == "accepted").count();
    assert_eq!(
        settled, 2,
        "both delivered operations are settled: {rows:?}"
    );
    assert_eq!(
        rows.iter()
            .filter(|(_, s, h)| s == "pending" && h.as_deref() == Some("skip_repos"))
            .count(),
        1,
        "the withheld one is still held, never settled: {rows:?}"
    );

    client.sync();
    assert_eq!(
        client.exchange_status()["protocol"],
        "replica-v1",
        "and it never reverts"
    );
}

#[test]
fn auth_and_protocol_failures_never_downgrade() {
    let mut hub = Hub::start();
    let client = approved_client(&hub);
    client.save("selected over replica before the key rotates", REPO);
    client.sync();
    assert_eq!(client.exchange_status()["protocol"], "replica-v1");

    std::fs::write(
        client.data_dir().join("config.toml"),
        "[sync]\ndaemon_interval = \"1s\"\n",
    )
    .expect("config");
    hub.restart(); // rotates the engine token: the stored credential now gets 401
    // Made while revoked: the save's inline push is refused, so it stays owed.
    client.save("owed while the credential is revoked", REPO);
    let mut daemon = client.spawn(&["sync", "daemon", "run"], &[]);
    std::thread::sleep(Duration::from_secs(2));
    let requests_at_suspension = hub.proxy.log().len();
    std::thread::sleep(Duration::from_secs(4));

    assert!(
        daemon.try_wait().expect("poll daemon").is_none(),
        "the daemon stays up"
    );
    let status = client.exchange_status();
    assert_eq!(status["network"], "auth_suspended", "{status}");
    assert_eq!(
        status["protocol"], "replica-v1",
        "a 401 never changes the selection"
    );
    assert_eq!(
        status["outbox"]["pending"], 1,
        "the owed save stays visible"
    );
    assert_eq!(
        hub.proxy.log().len(),
        requests_at_suspension,
        "three more cycles made no request while suspended"
    );

    client.login(&hub); // a new credential is the state change that resumes
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while client.exchange_status()["outbox"]["pending"] != 0 && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(300));
    }
    assert_eq!(client.exchange_status()["network"], "ok");
    assert_eq!(
        client.exchange_status()["outbox"]["pending"],
        0,
        "the same daemon drained it"
    );
    daemon.kill().expect("kill daemon");
    let _ = daemon.wait();

    // A corrupted protocol answer is a protocol error, never a reason to fall
    // back — neither for a fresh key nor for one already on replica-v1.
    for already_selected in [false, true] {
        let hub = Hub::start();
        let client = approved_client(&hub);
        client.save("protocol failures never downgrade", REPO);
        if already_selected {
            client.sync();
        }
        hub.proxy.arm(Fault::FlipResponse {
            path: "/sync/replica/manifest".into(),
            needle: "replica-v1".into(),
            replacement: "replica-v9".into(),
        });
        let before = hub.proxy.log().len();
        let _ = client.cli_raw(&["sync"], &[]);
        let status = client.exchange_status();
        assert_eq!(status["network"], "protocol_error", "{status}");
        assert_ne!(status["protocol"], "legacy", "never a silent downgrade");
        let after: Vec<String> = hub.proxy.log()[before..]
            .iter()
            .map(|l| l.path.clone())
            .collect();
        assert!(
            after
                .iter()
                .all(|p| p.contains("/v1/sync/status") || p.contains("/sync/replica/manifest")),
            "nothing but negotiation was attempted after the corrupt answer: {after:?}"
        );
    }
}
