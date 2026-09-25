#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The exchange client (#255), part 2: draining a large durable backlog in
//! one run (AC-4), and a budgeted daemon pass that reschedules without
//! sleeping, yields to an inline save, and survives `SIGTERM`/`SIGKILL`
//! (AC-5).
//!
//! Real `comemory serve` hubs, the real CLI binary, real SQLite, real HTTP
//! through the fault proxy's request log (never a mocked clock).

#[path = "common/exchange_support.rs"]
mod exchange_support;
#[path = "common/fault_proxy.rs"]
mod fault_proxy;
#[path = "common/replica_support.rs"]
mod replica_support;

use std::collections::BTreeSet;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use exchange_support::{Client, Hub, REPO, WORKSPACE, guide_body};
use serde_json::json;

/// A client logged into `hub` with `REPO` approved.
fn approved_client(hub: &Hub) -> Client {
    let client = Client::new();
    client.login(hub);
    client.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    client
}

/// The `limit=` query value of one request target, when it carries one.
fn limit_param(path: &str) -> Option<u64> {
    let query = path.split('?').nth(1)?;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("limit="))
        .and_then(|v| v.parse().ok())
}

/// Live memory count in `conn`.
fn live_count(conn: &rusqlite::Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE deleted_at IS NULL",
        [],
        |r| r.get(0),
    )
    .expect("live count")
}

/// Poll `client`'s status until `caught_up` or `deadline` passes, failing
/// the test if `daemon` exits first. Returns whether it caught up.
fn wait_caught_up(client: &Client, daemon: &mut Child, deadline: Instant) -> bool {
    while Instant::now() < deadline {
        if let Ok(Some(status)) = daemon.try_wait() {
            panic!("the daemon exited before catching up: {status}");
        }
        if client.exchange_status()["caught_up"] == true {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

/// The daemon can hold the hub's write permit while pushing B's inline save.
/// Retry only the documented busy response; the identical content-addressed
/// request is idempotent, and the caller's overall drain deadline still applies.
fn save_during_drain(hub: &Hub, n: usize) -> String {
    let request = json!({"body": guide_body(n), "kind": "decision", "repo": REPO});
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (status, body) = hub.engine().post("/api/v1/memories", &request);
        if status == 503 && body["error"]["code"] == "busy" && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        assert!(status < 300, "extra hub save {n}: {status} {body}");
        return body["data"]["id"].as_str().expect("id").to_string();
    }
}

// ---------------------------------------------------------------------------
// AC-4: a large durable backlog drains in one client run plus one nudge.
// ---------------------------------------------------------------------------

#[test]
fn large_backlog_drains_on_one_run() {
    let hub = Hub::start();
    let a = approved_client(&hub);
    let a_serve = a.serve();

    // 1,500 real saves through A's own HTTP API, built from this repository's
    // guides.
    let mut ids: Vec<String> = Vec::with_capacity(1_500);
    for n in 0..1_500 {
        let (status, body) = a_serve.post(
            "/api/v1/memories",
            &json!({"body": guide_body(n), "kind": "decision", "repo": REPO}),
        );
        assert!(status < 300, "save {n}: {status} {body}");
        ids.push(body["data"]["id"].as_str().expect("id").to_string());
    }

    // 600 frontmatter (tag) patches over a subset of those saves — more than
    // 2,000 operations in total, none of them content edits.
    let mut last_patch: Option<(String, Vec<String>)> = None;
    for (n, id) in ids.iter().take(600).cloned().enumerate() {
        let tags = vec![format!("patch-{n:04}")];
        let (status, body) =
            a_serve.patch(&format!("/api/v1/memories/{id}"), &json!({"tags": tags}));
        assert!(status < 300, "patch {n} of {id}: {status} {body}");
        last_patch = Some((id, tags));
    }
    let (last_id, last_tags) = last_patch.expect("at least one patch was made");

    // A's own serve drops before the drain: the backlog must come from the
    // durable outbox, not a live connection.
    drop(a_serve);

    a.sync();
    let a_status = a.exchange_status();
    assert_eq!(
        a_status["outbox"]["pending"], 0,
        "one `comemory sync` on A drains the whole backlog: {a_status}"
    );

    let a_ids: BTreeSet<String> = a.memory_ids().into_iter().collect();
    assert!(
        a_ids.len() >= 1_500,
        "A itself kept every live memory: {}",
        a_ids.len()
    );

    let b = approved_client(&hub);
    let before = hub.proxy.log().len();
    b.cli(&["sync", "--action", "auto"]);

    let b_ids: BTreeSet<String> = b.memory_ids().into_iter().collect();
    assert_eq!(
        b_ids, a_ids,
        "B mirrors every live memory A pushed, and nothing else"
    );

    let tags_on_b: Vec<String> = {
        let conn = b.open();
        let mut statement = conn
            .prepare("SELECT tag FROM memory_tags WHERE memory_id = ?1 ORDER BY tag")
            .expect("prepare");
        statement
            .query_map([&last_id], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect")
    };
    assert_eq!(
        tags_on_b, last_tags,
        "B carries the last patch's tags for {last_id}"
    );

    let b_status = b.exchange_status();
    assert_eq!(
        b_status["applied_sequence"],
        hub.head(),
        "B's cursor reaches the upstream head: {b_status}"
    );
    assert_eq!(b_status["caught_up"], true, "{b_status}");

    let changes_requests: Vec<_> = hub
        .proxy
        .log()
        .into_iter()
        .skip(before)
        .filter(|l| l.path.contains("/sync/replica/changes"))
        .collect();
    assert!(
        changes_requests.len() >= 4,
        "the 2,100-operation backlog paged over several requests, not one: {}",
        changes_requests.len()
    );
    for request in &changes_requests {
        let limit = limit_param(&request.path);
        assert!(
            limit.is_some_and(|l| l <= 500),
            "no page asked for more than 500 entries: {}",
            request.path
        );
    }
}

// ---------------------------------------------------------------------------
// AC-5: a budgeted pass reschedules itself and yields to concurrent work.
// ---------------------------------------------------------------------------

#[test]
fn budgeted_pass_reschedules_and_yields() {
    let hub = Hub::start();

    // 3,000 upstream entries made directly through the hub's own HTTP API.
    for n in 0..3_000 {
        let (status, body) = hub.engine().post(
            "/api/v1/memories",
            &json!({"body": guide_body(n), "kind": "decision", "repo": REPO}),
        );
        assert!(status < 300, "hub save {n}: {status} {body}");
    }

    // The hub journalled every write, but it advertises replica-v1 only once
    // its seeding scan has walked them all; without that, B would rightly
    // negotiate the old protocol (upstream_not_ready).
    hub.finish_seeding();

    let budget_config = "[sync]\npass_budget = \"1s\"\ndaemon_interval = \"60s\"\n";

    // --- (a) a daemon with a 1s pass budget catches up without a nudge,
    // yields to a concurrent save, and picks up writes made mid-drain. ---
    let b = approved_client(&hub);
    std::fs::write(b.data_dir().join("config.toml"), budget_config).expect("write B config");

    let before_b = hub.proxy.log().len();
    let daemon_start = Instant::now();
    let mut daemon = b.spawn(&["sync", "daemon", "run"], &[]);

    let save_start = Instant::now();
    let saved_id = b.save(
        "a save made on B while its daemon drains a large backlog",
        REPO,
    );
    let save_elapsed = save_start.elapsed();
    assert!(
        save_elapsed < Duration::from_secs(5),
        "an inline save on B yields to the busy drain instead of blocking on it: {save_elapsed:?}"
    );

    // Entries written to the hub while the pass runs are drained by the pass
    // that follows it, with no nudge.
    std::thread::sleep(Duration::from_millis(300));
    let mut extra_ids: Vec<String> = Vec::with_capacity(20);
    for n in 0..20 {
        extra_ids.push(save_during_drain(&hub, 3_000 + n));
    }

    let overall_deadline = daemon_start + Duration::from_mins(2);
    let caught_up = wait_caught_up(&b, &mut daemon, overall_deadline);
    let elapsed = daemon_start.elapsed();
    assert!(
        caught_up,
        "B's daemon caught up on the 3,000-entry backlog within 120s of starting"
    );
    assert!(
        elapsed < Duration::from_secs(50),
        "a daemon whose passes end on budget never sleeps between them: caught up in {elapsed:?}"
    );

    let changes_after_start = hub
        .proxy
        .log()
        .into_iter()
        .skip(before_b)
        .filter(|l| l.path.contains("/sync/replica/changes"))
        .count();
    assert!(
        changes_after_start > 1,
        "a 3,000-entry backlog under a 1s budget took more than one changes request: {changes_after_start}"
    );

    let b_ids: BTreeSet<String> = b.memory_ids().into_iter().collect();
    for id in &extra_ids {
        assert!(
            b_ids.contains(id),
            "a hub write made mid-drain reached B without any nudge: {id} not in B's mirror"
        );
    }

    assert!(
        hub.feed()
            .iter()
            .any(|(_, entity_key, _)| entity_key == &saved_id),
        "B's save, made while the drain was busy, is accepted upstream by that same drain: {saved_id}"
    );

    let _ = daemon.kill();
    let _ = daemon.wait();

    // --- (b) SIGTERM mid-drain exits within 2s. ---
    let c = approved_client(&hub);
    std::fs::write(c.data_dir().join("config.toml"), budget_config).expect("write C config");

    let mut term_daemon = c.spawn(&["sync", "daemon", "run"], &[]);
    std::thread::sleep(Duration::from_secs(1));
    let pid = term_daemon.id();
    let sent = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("send SIGTERM");
    assert!(sent.success(), "kill -TERM {pid} was accepted");

    let term_deadline = Instant::now() + Duration::from_secs(2);
    let mut exited = false;
    while Instant::now() < term_deadline {
        if term_daemon
            .try_wait()
            .expect("poll after SIGTERM")
            .is_some()
        {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(exited, "a daemon sent SIGTERM mid-drain exits within 2s");
    let _ = term_daemon.kill();
    let _ = term_daemon.wait();

    // --- (c) SIGKILL mid-page: no lost, no duplicated effect on resume. ---
    let mut kill_daemon = c.spawn(&["sync", "daemon", "run"], &[]);
    std::thread::sleep(Duration::from_secs(1));
    kill_daemon.kill().expect("SIGKILL the drain mid-page");
    let _ = kill_daemon.wait();

    let mut resume_daemon = c.spawn(&["sync", "daemon", "run"], &[]);
    let resume_deadline = Instant::now() + Duration::from_secs(90);
    let resumed = wait_caught_up(&c, &mut resume_daemon, resume_deadline);
    assert!(
        resumed,
        "C catches up after a SIGKILL mid-page: {:?}",
        c.exchange_status()
    );
    let _ = resume_daemon.kill();
    let _ = resume_daemon.wait();

    let conn = c.open();
    let receipt_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM replica_receipt", [], |r| r.get(0))
        .expect("receipt count");
    let distinct_operations: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT operation_id) FROM replica_receipt",
            [],
            |r| r.get(0),
        )
        .expect("distinct receipt operations");
    let synced_feed_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM replica_feed WHERE origin = 'sync'",
            [],
            |r| r.get(0),
        )
        .expect("synced feed count");
    assert_eq!(
        receipt_count, distinct_operations,
        "every applied upstream operation left exactly one receipt"
    );
    assert_eq!(
        receipt_count, synced_feed_count,
        "a killed-and-resumed drain journals no duplicate effect"
    );

    let c_live = live_count(&conn);
    let hub_live = live_count(&hub.db());
    assert_eq!(
        c_live, hub_live,
        "C's live memory count matches the hub's after resume"
    );

    let markdown_files = std::fs::read_dir(c.data_dir().join("memories"))
        .expect("read C's memories dir")
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "md"))
        .count();
    assert_eq!(
        markdown_files as i64, c_live,
        "one markdown file per live memory, no duplicates and nothing missing"
    );
}
