#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The exchange client (#255), part 5: rate limits, a stopped upstream, a
//! response held past the request timeout, and a repository revoked while a
//! pull request is in flight (AC-11, AC-12).
//!
//! Real `comemory serve` hubs, the real CLI binary, real SQLite, real HTTP
//! through the fault proxy — no mocked clock, only real sleeps against a
//! `retry_at` the drain itself persists.

#[path = "common/exchange_support.rs"]
mod exchange_support;
#[path = "common/fault_proxy.rs"]
mod fault_proxy;
#[path = "common/replica_support.rs"]
mod replica_support;

use std::collections::BTreeMap;
use std::time::Duration;

use exchange_support::{Client, Hub, REPO, WORKSPACE};
use fault_proxy::{Fault, Logged, Outcome};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A client logged into `hub` with `repos` approved at `revision`.
fn login_and_approve(hub: &Hub, repos: &[&str], revision: i64) -> Client {
    let client = Client::new();
    client.login(hub);
    client.approve(&hub.api_url(), WORKSPACE, repos, revision);
    client
}

/// The operation id of the sole outbox row a client has right now — used
/// right after one `save` on a client that has pushed nothing yet, so the row
/// is unambiguously that save's own operation.
fn sole_op_id(client: &Client) -> String {
    let rows = client.outbox();
    assert_eq!(rows.len(), 1, "exactly one outbox row expected: {rows:?}");
    rows[0].0.clone()
}

/// Parse an RFC 3339 timestamp the status JSON carries.
fn parse_rfc3339(raw: &str) -> OffsetDateTime {
    OffsetDateTime::parse(raw, &Rfc3339).unwrap_or_else(|e| panic!("{raw} is not RFC 3339: {e}"))
}

/// `status.retry_at`, parsed.
fn retry_at_of(status: &Value) -> OffsetDateTime {
    let raw = status["retry_at"]
        .as_str()
        .unwrap_or_else(|| panic!("retry_at is not a string: {status}"));
    parse_rfc3339(raw)
}

/// Sleep past `retry_at` by `slack`, using a real clock read (never a fixed
/// long sleep guessed from the outside).
fn sleep_past(retry_at: OffsetDateTime, slack: Duration) {
    let remaining = retry_at - OffsetDateTime::now_utc();
    let millis = u64::try_from(remaining.whole_milliseconds().max(0)).unwrap_or(0);
    std::thread::sleep(Duration::from_millis(millis) + slack);
}

/// Whether the hub's feed carries `operation_id` at all, and how many times.
fn feed_count(hub: &Hub, operation_id: &str) -> usize {
    hub.feed()
        .iter()
        .filter(|(op, _, _)| op == operation_id)
        .count()
}

/// `BadGateway`-outcome log entries, grouped by request path.
fn bad_gateway_counts_by_path(entries: &[Logged]) -> BTreeMap<String, usize> {
    let mut by_path = BTreeMap::new();
    for entry in entries.iter().filter(|e| e.outcome == Outcome::BadGateway) {
        *by_path.entry(entry.path.clone()).or_insert(0) += 1;
    }
    by_path
}

// ---------------------------------------------------------------------------
// AC-11: rate limit, a stopped upstream, a response held past the timeout
// ---------------------------------------------------------------------------

#[test]
fn rate_limit_gateway_and_timeout() {
    // (a) 429 with Retry-After: the drain backs off exactly that long, then
    // completes with nothing lost.
    {
        let hub = Hub::start();
        let client = login_and_approve(&hub, &[REPO], 1);
        client.save("owed while the proxy refuses above its rate", REPO);
        let m1_op = sole_op_id(&client);
        hub.proxy.arm(Fault::RateLimit {
            max: 0,
            retry_after: 2,
        });

        let _ = client.cli_raw(&["sync", "--action", "auto"], &[]);
        let status = client.exchange_status();
        assert_eq!(status["network"], "backoff", "{status}");
        let retry_at = retry_at_of(&status);
        let now = OffsetDateTime::now_utc();
        assert!(
            retry_at >= now,
            "retry_at {retry_at} is not in the future ({now})"
        );
        assert!(
            retry_at <= now + time::Duration::seconds(3),
            "retry_at {retry_at} is further than now+3s ({now})"
        );

        hub.proxy.clear();
        let before = hub.proxy.log().len();
        let _ = client.cli_raw(&["sync", "--action", "auto"], &[]);
        assert_eq!(
            hub.proxy.log().len(),
            before,
            "a manual run inside Retry-After still makes no request"
        );

        sleep_past(retry_at, Duration::from_millis(300));
        let _ = client.cli_raw(&["sync", "--action", "auto"], &[]);
        let status = client.exchange_status();
        assert_eq!(
            status["outbox"]["pending"], 0,
            "the owed save finally drained: {status}"
        );
        assert_eq!(
            feed_count(&hub, &m1_op),
            1,
            "and reached the hub under its own id"
        );
    }

    // (b) 502 while the hub is stopped: bounded attempts per request, a
    // growing consecutive-failure count, no request before retry_at, and a
    // clean converge once the hub is back with a fresh credential.
    {
        let mut hub = Hub::start();
        let client = login_and_approve(&hub, &[REPO], 1);
        client.save("owed while the hub is unreachable", REPO);
        let m2_op = sole_op_id(&client);
        hub.stop();

        let before = hub.proxy.log().len();
        let _ = client.cli_raw(&["sync", "--action", "auto"], &[]);
        let new_entries = hub.proxy.log()[before..].to_vec();
        let by_path = bad_gateway_counts_by_path(&new_entries);
        assert!(
            !by_path.is_empty(),
            "at least one bad-gateway answer: {new_entries:?}"
        );
        for (path, count) in &by_path {
            assert!(
                (1..=3).contains(count),
                "{path} saw {count} attempts, expected 1..=3: {new_entries:?}"
            );
        }
        let status = client.exchange_status();
        assert_eq!(status["network"], "backoff", "{status}");
        assert_eq!(status["consecutive_failures"], 1, "{status}");
        let first_retry_at = retry_at_of(&status);
        let now = OffsetDateTime::now_utc();
        assert!(
            first_retry_at - now <= time::Duration::seconds(2) + time::Duration::milliseconds(500),
            "first retry_at {first_retry_at} is outside the <=2s (+0.5s slack) window from {now}"
        );

        let before = hub.proxy.log().len();
        let _ = client.cli_raw(&["sync", "--action", "auto"], &[]);
        assert_eq!(
            hub.proxy.log().len(),
            before,
            "still inside the backoff window, so no request went out"
        );

        sleep_past(first_retry_at, Duration::from_millis(300));
        let _ = client.cli_raw(&["sync", "--action", "auto"], &[]);
        let status = client.exchange_status();
        let now = OffsetDateTime::now_utc();
        assert_eq!(status["consecutive_failures"], 2, "{status}");
        let second_retry_at = retry_at_of(&status);
        assert!(
            second_retry_at - now <= time::Duration::seconds(4) + time::Duration::milliseconds(500),
            "second retry_at {second_retry_at} is outside the <=4s (+0.5s slack) window from {now}"
        );

        hub.restart();
        client.login(&hub); // a fresh credential
        sleep_past(second_retry_at, Duration::from_millis(300));
        let _ = client.cli_raw(&["sync", "--action", "auto"], &[]);
        let status = client.exchange_status();
        assert_eq!(
            status["outbox"]["pending"], 0,
            "the restarted hub converged: {status}"
        );
        assert_eq!(
            feed_count(&hub, &m2_op),
            1,
            "m2 reached the hub exactly once"
        );
    }

    // (c) A response held past request_timeout is retried under the same
    // operation id, and the retry lands once.
    {
        let hub = Hub::start();
        let client = login_and_approve(&hub, &[REPO], 1);
        std::fs::write(
            client.data_dir().join("config.toml"),
            "[sync]\nrequest_timeout = \"2s\"\npush_on_save = false\n",
        )
        .expect("config");
        client.save(
            "owed through a response held past the request timeout",
            REPO,
        );
        let m3_op = sole_op_id(&client);
        hub.proxy.arm(Fault::DelayResponse {
            path: "/sync/replica/import".into(),
            delay: Duration::from_secs(3),
            times: 1,
        });

        client.sync(); // may succeed after its in-pass retry

        let import_requests: Vec<Logged> = hub
            .proxy
            .log()
            .into_iter()
            .filter(|l| l.path.contains("/sync/replica/import") && l.body.contains(&m3_op))
            .collect();
        assert!(
            import_requests.len() >= 2,
            "the timed-out import is retried under the same operation id: {import_requests:?}"
        );
        assert_eq!(
            feed_count(&hub, &m3_op),
            1,
            "the hub applied it exactly once"
        );
        let accepted = client
            .outbox()
            .into_iter()
            .find(|(id, _, _)| id == &m3_op)
            .unwrap_or_else(|| panic!("m3's outbox row is missing"));
        assert_eq!(
            accepted.1, "accepted",
            "m3's outbox row settled: {accepted:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC-12: a repository revoked while a pull request is held in flight
// ---------------------------------------------------------------------------

#[test]
fn revocation_during_request_holds() {
    const PRIVATE: &str = "acme/private";

    let hub = Hub::start();
    let writer = login_and_approve(&hub, &[PRIVATE, REPO], 1);
    let wp = writer.save("private repo content, approved for now", PRIVATE);
    let wr = writer.save("public repo content, always approved", REPO);
    writer.sync();

    let b = login_and_approve(&hub, &[PRIVATE, REPO], 1);
    hub.proxy.arm(Fault::HoldRequest {
        path: "/sync/replica/changes".into(),
    });
    let mut child = b.spawn(&["--json", "sync"], &[]);

    assert!(
        hub.proxy.wait_held(Duration::from_secs(10)),
        "the pull is parked"
    );
    b.approve(&hub.api_url(), WORKSPACE, &[REPO], 2); // revoke PRIVATE mid-flight
    hub.proxy.clear(); // releases the held request

    let _ = child.wait().expect("wait for the spawned sync"); // rows below, never the exit code

    let ids = b.memory_ids();
    assert!(
        ids.contains(&wr),
        "the still-approved repo's entry is applied: {ids:?}"
    );
    assert!(
        !ids.contains(&wp),
        "the revoked repo's entry is held, not applied: {ids:?}"
    );
    let exchange = b.exchange_status();
    assert_eq!(exchange["pull"]["held"]["policy"], 1, "{exchange}");

    // Push side: nothing for the revoked repo goes out while it stays revoked.
    b.save("owed under a repo that is currently revoked", PRIVATE);
    let bp_op = sole_op_id(&b);
    b.sync();
    let leaked = hub
        .proxy
        .log()
        .into_iter()
        .filter(|l| l.path.contains("/sync/replica/import"))
        .any(|l| l.body.contains(&bp_op));
    assert!(
        !leaked,
        "no import request carried the revoked repo's operation"
    );
    let bp_row = b
        .outbox()
        .into_iter()
        .find(|(id, _, _)| id == &bp_op)
        .unwrap_or_else(|| panic!("bp's outbox row is missing"));
    assert_eq!(
        bp_row.1, "pending",
        "bp stays pending while held: {bp_row:?}"
    );
    assert_eq!(
        bp_row.2.as_deref(),
        Some("policy"),
        "bp is held policy: {bp_row:?}"
    );

    // Restore: the held pull entry applies and the held push goes out.
    b.approve(&hub.api_url(), WORKSPACE, &[REPO, PRIVATE], 3);
    b.sync();
    let ids = b.memory_ids();
    assert!(
        ids.contains(&wp),
        "the restored repo's pulled entry is now applied: {ids:?}"
    );
    assert_eq!(
        feed_count(&hub, &bp_op),
        1,
        "bp's push went out once approval was restored"
    );
}

// ---------------------------------------------------------------------------
// The anchor read is a request like any other: its failure is a network end.
// ---------------------------------------------------------------------------

#[test]
fn a_failed_anchor_read_is_a_network_failure_not_caught_up() {
    let hub = Hub::start();
    let writer = login_and_approve(&hub, &[REPO], 1);
    writer.save("the entry a caught-up reader's cursor is left on", REPO);
    writer.sync();
    let reader = login_and_approve(&hub, &[REPO], 1);
    reader.sync();
    assert_eq!(reader.exchange_status()["caught_up"], true);

    // Caught up with nothing to send, the only `changes` read of the next
    // pass is the anchor check; lose its answer on every attempt.
    hub.proxy.arm(Fault::DropResponse {
        path: "/sync/replica/changes".into(),
        times: 3,
    });
    let (code, _, stderr) = reader.cli_raw(&["sync"], &[]);

    assert_eq!(code, 69, "the run reports the network failure: {stderr}");
    let status = reader.exchange_status();
    assert_eq!(status["network"], "backoff", "{status}");
    assert_eq!(status["caught_up"], false, "{status}");
}
