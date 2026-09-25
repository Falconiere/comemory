#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The exchange client (#255), part 4: a dropped acknowledgement plus a kill
//! must never lose, duplicate or reorder an operation id (AC-9), and a store
//! full of held rows — policy, secret, `skip_repos`, an unadvertised kind —
//! must keep pushing whatever IS eligible, never starve it, and survive a
//! restart and a rebuild with every hold and count intact (AC-10).
//!
//! Real `comemory serve` hubs, the real CLI binary, real SQLite, real HTTP.

#[path = "common/exchange_support.rs"]
mod exchange_support;
#[path = "common/fault_proxy.rs"]
mod fault_proxy;
#[path = "common/replica_support.rs"]
mod replica_support;

use exchange_support::{Client, Hub, REPO, WORKSPACE};
use fault_proxy::Fault;

// ---------------------------------------------------------------------------
// Suite-local helpers: everything a case needs to read state the harness
// itself does not expose (attempts, entity bindings, tag rows, feed order).
// ---------------------------------------------------------------------------

/// Every pending outbox operation id for `client`, oldest first.
fn pending_ids(client: &Client) -> Vec<String> {
    client
        .outbox()
        .into_iter()
        .filter(|(_, state, _)| state == "pending")
        .map(|(id, _, _)| id)
        .collect()
}

/// `client`'s state for one operation id, if it has one.
fn state_of(client: &Client, operation_id: &str) -> Option<String> {
    client
        .outbox()
        .into_iter()
        .find(|(id, _, _)| id == operation_id)
        .map(|(_, state, _)| state)
}

/// `replica_operation.attempts` for one row.
fn attempts_of(client: &Client, operation_id: &str) -> i64 {
    client
        .open()
        .query_row(
            "SELECT attempts FROM replica_operation WHERE operation_id = ?1",
            [operation_id],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("no attempts for {operation_id}: {e}"))
}

/// The most recent operation id journalled for one entity.
fn op_for_entity(client: &Client, entity_kind: &str, entity_key: &str) -> String {
    client
        .open()
        .query_row(
            "SELECT operation_id FROM replica_operation \
             WHERE entity_kind = ?1 AND entity_key = ?2 \
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            rusqlite::params![entity_kind, entity_key],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("no operation for {entity_kind}/{entity_key}: {e}"))
}

/// Every operation id journalled for one entity kind, oldest first.
fn op_ids_for_kind(client: &Client, entity_kind: &str) -> Vec<String> {
    let conn = client.open();
    let mut statement = conn
        .prepare(
            "SELECT operation_id FROM replica_operation \
             WHERE entity_kind = ?1 ORDER BY created_at, rowid",
        )
        .expect("prepare");
    statement
        .query_map([entity_kind], |r| r.get(0))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

/// Every currently held (still-pending, `hold_reason` set) operation id of
/// the kinds this suite writes — not the activity events its runs queue.
fn held_op_ids(client: &Client) -> Vec<String> {
    let conn = client.open();
    let mut statement = conn
        .prepare(
            "SELECT operation_id FROM replica_operation \
             WHERE state = 'pending' AND hold_reason IS NOT NULL \
             AND entity_kind NOT IN ('feedback_event', 'activity_event') \
             ORDER BY created_at, rowid",
        )
        .expect("prepare");
    statement
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

/// True when no held row has ever been marked anything but `pending`.
fn held_rows_all_pending(client: &Client) -> bool {
    let count: i64 = client
        .open()
        .query_row(
            "SELECT COUNT(*) FROM replica_operation \
             WHERE hold_reason IS NOT NULL AND state != 'pending'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    count == 0
}

/// The live tags of one memory, in this client's own mirror.
fn tags_of(client: &Client, memory_id: &str) -> Vec<String> {
    let conn = client.open();
    let mut statement = conn
        .prepare("SELECT tag FROM memory_tags WHERE memory_id = ?1 ORDER BY tag")
        .expect("prepare");
    statement
        .query_map([memory_id], |r| r.get(0))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

/// The 0-based position of `operation_id` in the hub's feed, oldest first.
fn feed_position(hub: &Hub, operation_id: &str) -> usize {
    hub.feed()
        .iter()
        .position(|(op, _, _)| op == operation_id)
        .unwrap_or_else(|| panic!("{operation_id} never reached the hub feed"))
}

/// How many feed positions carry `operation_id`.
fn feed_occurrences(hub: &Hub, operation_id: &str) -> usize {
    hub.feed()
        .iter()
        .filter(|(op, _, _)| op == operation_id)
        .count()
}

// ---------------------------------------------------------------------------
// AC-9: a dropped acknowledgement plus a kill keeps ids and order
// ---------------------------------------------------------------------------

#[test]
fn dropped_ack_and_kill_keep_ids_and_order() {
    let hub = Hub::start();
    let a = Client::new();
    a.login(&hub);
    a.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    let m = a.save(
        "Initial record of how comemory retries a push after a dropped ack.",
        REPO,
    );
    a.sync();

    let a_serve = a.serve();
    let (status1, _) = a_serve.patch(
        &format!("/api/v1/memories/{m}"),
        &serde_json::json!({"tags": ["p1"]}),
    );
    assert!(status1 < 300, "first patch failed: {status1}");
    let (status2, _) = a_serve.patch(
        &format!("/api/v1/memories/{m}"),
        &serde_json::json!({"tags": ["p1", "p2"]}),
    );
    assert!(status2 < 300, "second patch failed: {status2}");
    let pending_after_patches = pending_ids(&a);
    assert_eq!(
        pending_after_patches.len(),
        2,
        "two ops queued: {pending_after_patches:?}"
    );
    let op1 = pending_after_patches[0].clone();
    let op2 = pending_after_patches[1].clone();

    // Every attempt of the pass loses its answer (the attempt and its two
    // in-pass retries), as for a client killed right after the dropped ack.
    hub.proxy.arm(Fault::DropResponse {
        path: "/sync/replica/import".into(),
        times: 3,
    });
    let _ = a.cli_raw(&["sync"], &[]);
    assert_eq!(
        pending_ids(&a),
        vec![op1.clone(), op2.clone()],
        "nothing was acknowledged locally"
    );
    assert!(attempts_of(&a, &op1) >= 1, "op1 was attempted");
    assert!(attempts_of(&a, &op2) >= 1, "op2 was attempted");

    // Simulate "killed": the process that made that attempt already ended.
    // A clean sync now drains the same ids.
    a.sync();
    assert_eq!(feed_occurrences(&hub, &op1), 1, "op1 lands exactly once");
    assert_eq!(feed_occurrences(&hub, &op2), 1, "op2 lands exactly once");
    assert!(
        feed_position(&hub, &op1) < feed_position(&hub, &op2),
        "the order made is the order kept"
    );
    assert_eq!(state_of(&a, &op1), Some("accepted".to_string()));
    assert_eq!(state_of(&a, &op2), Some("accepted".to_string()));

    // Late-ack: a dropped response leaves the pending op recognized later,
    // by pull, while a newer local edit is never reverted.
    hub.proxy.arm(Fault::DropResponse {
        path: "/sync/replica/import".into(),
        times: 3,
    });
    let (status3, _) = a_serve.patch(
        &format!("/api/v1/memories/{m}"),
        &serde_json::json!({"tags": ["p3"]}),
    );
    assert!(status3 < 300, "p3 patch failed: {status3}");
    let op3 = pending_ids(&a).into_iter().next().expect("op3 minted");
    let _ = a.cli_raw(&["sync"], &[]);
    assert_eq!(
        state_of(&a, &op3),
        Some("pending".to_string()),
        "op3's ack was dropped"
    );

    let (status4, _) = a_serve.patch(
        &format!("/api/v1/memories/{m}"),
        &serde_json::json!({"tags": ["p4"]}),
    );
    assert!(status4 < 300, "p4 patch failed: {status4}");
    let op4 = pending_ids(&a)
        .into_iter()
        .last()
        .expect("op4 minted after op3");
    assert_ne!(op3, op4, "p4 is a distinct, newer operation");

    a.sync();
    assert_eq!(
        state_of(&a, &op3),
        Some("accepted".to_string()),
        "the late ack settled it"
    );
    assert_eq!(
        state_of(&a, &op4),
        Some("accepted".to_string()),
        "and the newer patch went out too"
    );
    assert_eq!(
        tags_of(&a, &m),
        vec!["p4".to_string()],
        "never reverted to p3"
    );

    // Peer patch while pending: a peer's edit pulled while A's own edit is
    // still pending never overwrites A's local file.
    let b = Client::new();
    b.login(&hub);
    b.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    b.sync();
    assert_eq!(
        tags_of(&b, &m),
        vec!["p4".to_string()],
        "B pulled A's latest"
    );

    let (status5, _) = a_serve.patch(
        &format!("/api/v1/memories/{m}"),
        &serde_json::json!({"tags": ["a5"]}),
    );
    assert!(status5 < 300, "a5 patch failed: {status5}");
    let op_a5 = pending_ids(&a).into_iter().next().expect("op for a5");
    // A does NOT sync here — its patch stays pending locally.

    let b_serve = b.serve();
    let (status6, _) = b_serve.patch(
        &format!("/api/v1/memories/{m}"),
        &serde_json::json!({"tags": ["b1"]}),
    );
    assert!(status6 < 300, "b1 patch failed: {status6}");
    let peer_patch_op = pending_ids(&b).into_iter().next().expect("op for b1");
    b.sync();
    assert_eq!(
        state_of(&b, &peer_patch_op),
        Some("accepted".to_string()),
        "b1 landed at the hub"
    );

    a.sync();
    assert_eq!(
        tags_of(&a, &m),
        vec!["a5".to_string()],
        "A's pending patch was never overwritten by the peer's pulled edit"
    );
    assert_eq!(
        state_of(&a, &op_a5),
        Some("accepted".to_string()),
        "A's own patch went out"
    );
    assert!(
        feed_position(&hub, &peer_patch_op) < feed_position(&hub, &op_a5),
        "server order: b1 before a5"
    );

    b.sync();
    assert_eq!(
        tags_of(&b, &m),
        vec!["a5".to_string()],
        "both machines converge on A's patch, in server order"
    );
}

// ---------------------------------------------------------------------------
// AC-10: held states never starve an eligible repository
// ---------------------------------------------------------------------------

#[test]
fn held_states_do_not_starve_eligible_repos() {
    let hub = Hub::start();
    let a = Client::new();
    a.login(&hub);
    a.approve(&hub.api_url(), WORKSPACE, &[REPO, "acme/skip"], 1);
    std::fs::write(
        a.data_dir().join("config.toml"),
        "[sync]\nskip_repos = [\"acme/skip\"]\npush_on_save = false\n",
    )
    .expect("config");

    let m_ok = a.save(
        "Coverage note: a push still drains when nothing about it is held.",
        REPO,
    );
    let m_private = a.save(
        "Notes on the private rollout this device has not been approved for.",
        "acme/private",
    );
    // `generic-high-entropy` in src/utilities/rules.toml: a `key`/`token`/
    // `secret`/`password`/`auth` label followed by `=`/`:` and 20+ opaque
    // characters. High-entropy but synthetic — not a format the repository's
    // own secret-content guardrail (AWS/GitHub/Slack/Stripe/PEM literals)
    // recognizes, so this fixture never becomes a committed credential.
    // Assembled at run time so no scanner sees a key-shaped literal.
    let m_secret = a.save(
        &format!(
            "Rotated the deploy credential: api_key={}{} (rotate before release).",
            "Zx9Qp2Lm7Rt4Wn8Y", "c3Vb6Hj1Ks5Fd0Ae"
        ),
        REPO,
    );
    let m_skip = a.save(
        "A memory filed under the repository label this device skips.",
        "acme/skip",
    );

    // Documents are shared only from under the label's indexed root: index a
    // small pinned checkout as REPO first, then the docs copied inside it.
    let docs_root = tempfile::tempdir().expect("docs root");
    let checkout = replica_support::pinned_repo(docs_root.path(), 3);
    replica_support::git(
        &checkout,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/Falconiere/comemory.git",
        ],
    );
    a.cli(&[
        "index-code",
        "--repo",
        REPO,
        "--path",
        checkout.to_str().expect("utf8 path"),
    ]);
    let docs_repo = replica_support::docs_tree(docs_root.path(), "pinned-repo");
    a.cli(&[
        "index",
        docs_repo.join("docs/guides").to_str().expect("utf8 path"),
        "--repo",
        REPO,
    ]);

    let m_ok_op = op_for_entity(&a, "memory", &m_ok);
    let m_private_op = op_for_entity(&a, "memory", &m_private);
    let m_secret_op = op_for_entity(&a, "memory", &m_secret);
    let m_skip_op = op_for_entity(&a, "memory", &m_skip);
    let doc_ops = op_ids_for_kind(&a, "document_revision");
    assert!(
        !doc_ops.is_empty(),
        "indexing journalled at least one document revision"
    );

    hub.proxy.arm(Fault::FlipResponse {
        path: "/sync/replica/manifest".into(),
        needle: "document_revision@1".into(),
        replacement: "document_revision@9".into(),
    });
    a.sync();

    assert_eq!(
        state_of(&a, &m_ok_op),
        Some("accepted".to_string()),
        "the eligible one flowed"
    );
    assert!(
        feed_occurrences(&hub, &m_ok_op) >= 1,
        "m_ok reached the hub feed"
    );

    let status = a.exchange_status();
    assert_eq!(status["outbox"]["held"]["policy"], 1, "m_private: {status}");
    assert_eq!(status["outbox"]["held"]["secret"], 1, "{status}");
    assert_eq!(
        status["outbox"]["held"]["skip_repos"], 2,
        "m_skip and the run that saved it: {status}"
    );
    assert_eq!(
        status["outbox"]["held"]["incompatible"],
        doc_ops.len(),
        "every document revision: {status}"
    );

    let held_ids = held_op_ids(&a);
    assert!(
        held_ids.contains(&m_private_op),
        "m_private held: {held_ids:?}"
    );
    assert!(
        held_ids.contains(&m_secret_op),
        "m_secret held: {held_ids:?}"
    );
    assert!(held_ids.contains(&m_skip_op), "m_skip held: {held_ids:?}");
    for doc_op in &doc_ops {
        assert!(held_ids.contains(doc_op), "{doc_op} held: {held_ids:?}");
    }
    assert!(
        held_rows_all_pending(&a),
        "no held row was ever marked settled"
    );

    for logged in hub.proxy.requests_to("/sync/replica/import") {
        for id in &held_ids {
            assert!(
                !logged.body.contains(id.as_str()),
                "held operation {id} leaked into an import request: {}",
                logged.body
            );
        }
    }

    // A later approval sends only the newly eligible operation.
    a.approve(
        &hub.api_url(),
        WORKSPACE,
        &[REPO, "acme/skip", "acme/private"],
        2,
    );
    hub.proxy.arm(Fault::FlipResponse {
        path: "/sync/replica/manifest".into(),
        needle: "document_revision@1".into(),
        replacement: "document_revision@9".into(),
    });
    let before_len = hub.proxy.log().len();
    a.sync();

    assert_eq!(
        state_of(&a, &m_private_op),
        Some("accepted".to_string()),
        "the newly approved repository's operation went out"
    );
    let new_import_bodies: Vec<String> = hub.proxy.log()[before_len..]
        .iter()
        .filter(|l| l.path.contains("/sync/replica/import"))
        .map(|l| l.body.clone())
        .collect();
    assert!(
        new_import_bodies
            .iter()
            .any(|b| b.contains(m_private_op.as_str())),
        "m_private's operation went out: {new_import_bodies:?}"
    );
    for id in [&m_secret_op, &m_skip_op] {
        assert!(
            new_import_bodies.iter().all(|b| !b.contains(id.as_str())),
            "{id} is still held, never sent: {new_import_bodies:?}"
        );
    }
    for doc_op in &doc_ops {
        assert!(
            new_import_bodies
                .iter()
                .all(|b| !b.contains(doc_op.as_str())),
            "{doc_op} is still incompatible, never sent: {new_import_bodies:?}"
        );
    }

    // Retryable: a row whose import answer is lost on every attempt of the
    // pass stays pending, counted.
    hub.proxy.arm(Fault::FlipResponse {
        path: "/sync/replica/manifest".into(),
        needle: "document_revision@1".into(),
        replacement: "document_revision@9".into(),
    });
    hub.proxy.arm(Fault::DropResponse {
        path: "/sync/replica/import".into(),
        times: 3,
    });
    let m_late = a.save(
        "Filed while the hub is unreachable, to prove retryable survives a restart.",
        REPO,
    );
    let _ = a.cli_raw(&["sync"], &[]);
    let status = a.exchange_status();
    assert_eq!(
        status["outbox"]["retryable"], 2,
        "m_late and the run that saved it, both in the lost batch: {status}"
    );
    let m_late_op = op_for_entity(&a, "memory", &m_late);
    assert_eq!(state_of(&a, &m_late_op), Some("pending".to_string()));

    // Durability: the held and retryable picture survives a rebuild.
    let before_outbox = a.exchange_status()["outbox"].clone();
    a.cli(&["rebuild"]);
    let after_outbox = a.exchange_status()["outbox"].clone();
    assert_eq!(
        before_outbox, after_outbox,
        "held and retryable counts are unchanged by a rebuild"
    );

    hub.proxy.clear();
    a.sync();
    assert_eq!(
        state_of(&a, &m_late_op),
        Some("accepted".to_string()),
        "the retryable row settles once the answers come back"
    );
    assert_eq!(feed_occurrences(&hub, &m_late_op), 1, "and it landed once");
}

// ---------------------------------------------------------------------------
// An operation the upstream leaves unanswered is sent at most twice per pass.
// ---------------------------------------------------------------------------

#[test]
fn an_operation_left_unanswered_rests_until_the_next_pass() {
    let hub = Hub::start();
    let a = Client::new();
    a.login(&hub);
    a.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    std::fs::write(
        a.data_dir().join("config.toml"),
        "[sync]\npush_on_save = false\n",
    )
    .expect("config");
    a.save(
        "An operation whose acknowledgement names another id, over and over.",
        REPO,
    );
    let op = pending_ids(&a).pop().expect("one owed operation");
    // Five one-shot flips: each import answer that follows names a different
    // operation id, so this operation is answered by nobody. The engine still
    // applies it the first time; every later send is a duplicate there.
    let stranger = format!(
        "{}{}",
        &op[..op.len() - 1],
        if op.ends_with('0') { '1' } else { '0' }
    );
    for _ in 0..5 {
        hub.proxy.arm(Fault::FlipResponse {
            path: "/sync/replica/import".into(),
            needle: op.clone(),
            replacement: stranger.clone(),
        });
    }

    let (code, _, stderr) = a.cli_raw(&["sync"], &[]);

    assert_eq!(
        code, 0,
        "a manual run ends even though no answer came: {stderr}"
    );
    let sends = hub
        .proxy
        .requests_to("/sync/replica/import")
        .into_iter()
        .filter(|l| l.body.contains(&op))
        .count();
    assert_eq!(
        sends, 2,
        "sent twice without an answer, then left for the next pass"
    );
    assert_eq!(state_of(&a, &op), Some("pending".to_string()));
    assert_eq!(attempts_of(&a, &op), 2);

    hub.proxy.clear();
    a.sync();
    assert_eq!(
        state_of(&a, &op),
        Some("accepted".to_string()),
        "the next pass settles it — its own operation, recognized in the pulled feed"
    );
    assert_eq!(feed_occurrences(&hub, &op), 1, "and the hub holds it once");
}
