#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The exchange client (#255), part 3: what a pull does under three ways it
//! cannot advance cleanly through a page — several full pages a repository
//! policy holds before one allowed entry, a real write failure mid-page, and
//! an entry of a kind/version this build cannot read (AC-6, AC-7, AC-8) —
//! plus the push-side half of AC-7: a batch the hub answered partly
//! `accepted` and partly `rejected_stale`, whose response was dropped, is
//! resent whole and every operation gets its original answer back.
//!
//! Real `comemory serve` hubs, the real CLI binary, real SQLite, real HTTP.

#[path = "common/exchange_support.rs"]
mod exchange_support;
#[path = "common/fault_proxy.rs"]
mod fault_proxy;
#[path = "common/replica_support.rs"]
mod replica_support;

use exchange_support::{Client, Hub, REPO, WORKSPACE, guide_body};
use fault_proxy::Fault;
use serde_json::{Value, json};

/// A client logged into `hub` with `REPO` approved at revision 1.
fn approved_client(hub: &Hub) -> Client {
    let client = Client::new();
    client.login(hub);
    client.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    client
}

/// Parse a raw CLI run's stdout as JSON. A run that printed nothing
/// parseable reads as `Value::Null`, which is its own red signal against the
/// assertions below rather than a panic here.
fn parse_json(stdout: &str) -> Value {
    serde_json::from_str(stdout).unwrap_or(Value::Null)
}

/// The hub feed's `(operation_id, sequence)` for one entity. Every entity
/// these suites build is written once, so one row is the only row.
fn hub_feed_row(hub: &Hub, entity_key: &str) -> (String, i64) {
    hub.db()
        .query_row(
            "SELECT operation_id, sequence FROM replica_feed WHERE entity_key = ?1 \
             ORDER BY sequence LIMIT 1",
            [entity_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or_else(|e| panic!("no hub feed row for {entity_key}: {e}"))
}

/// This client's persisted cursor position for its one session key.
fn applied_sequence(client: &Client) -> i64 {
    client
        .open()
        .query_row("SELECT applied_sequence FROM replica_cursor", [], |r| {
            r.get(0)
        })
        .unwrap_or_else(|e| panic!("no replica_cursor row: {e}"))
}

/// How many `replica_receipt` rows this client holds for `operation_id`.
fn receipt_count(client: &Client, operation_id: &str) -> i64 {
    client
        .open()
        .query_row(
            "SELECT COUNT(*) FROM replica_receipt WHERE operation_id = ?1",
            [operation_id],
            |r| r.get(0),
        )
        .expect("count receipts")
}

/// How many `replica_feed` rows this client journalled for `entity_key` under
/// `origin = 'sync'` — a pulled entry's own journal entry.
fn synced_feed_rows(client: &Client, entity_key: &str) -> i64 {
    client
        .open()
        .query_row(
            "SELECT COUNT(*) FROM replica_feed WHERE entity_key = ?1 AND origin = 'sync'",
            [entity_key],
            |r| r.get(0),
        )
        .expect("count synced feed rows")
}

/// How many markdown files this client's `memories/` directory holds whose
/// name starts with `id` — `save` names files `{id}-{slug}.md`.
fn markdown_files(client: &Client, id: &str) -> usize {
    std::fs::read_dir(client.data_dir().join("memories"))
        .expect("read memories dir")
        .filter_map(std::result::Result::ok)
        .filter(|e| {
            e.file_name().to_str().is_some_and(|n| {
                n.starts_with(id)
                    && std::path::Path::new(n)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            })
        })
        .count()
}

// ---------------------------------------------------------------------------
// AC-6: pages held by policy, then an allowed entry, then approval
// ---------------------------------------------------------------------------

#[test]
fn held_pages_then_allowed_entry() {
    let hub = Hub::start();

    let writer = Client::new();
    writer.login(&hub);
    writer.approve(&hub.api_url(), WORKSPACE, &["acme/private", REPO], 1);
    let serve = writer.serve();
    for n in 0..1500 {
        let (status, body) = serve.post(
            "/api/v1/memories",
            &json!({"body": guide_body(n), "kind": "decision", "repo": "acme/private"}),
        );
        assert!(status < 300, "writer save {n}: {status} {body}");
    }
    let (status, allowed) = serve.post(
        "/api/v1/memories",
        &json!({
            "body": "the one entry that lands under the reader's own approved repository",
            "kind": "decision",
            "repo": REPO,
        }),
    );
    assert!(
        status < 300,
        "writer save (approved repo): {status} {allowed}"
    );
    let allowed_id = allowed["data"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no id: {allowed}"))
        .to_string();
    drop(serve);
    writer.sync();

    let reader = Client::new();
    reader.login(&hub);
    reader.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    reader.sync();

    let ids = reader.memory_ids();
    assert!(
        ids.contains(&allowed_id),
        "the approved entry applied: {ids:?}"
    );
    assert_eq!(
        ids.len(),
        1,
        "none of the unapproved repository's memories landed: {}",
        ids.len()
    );
    let status = reader.exchange_status();
    // The 1,500 memories, and the 1,500 runs that saved them, which the
    // writer's sync shared under the same repository.
    assert_eq!(status["pull"]["held"]["policy"], 3000, "{status}");
    assert_eq!(status["applied_sequence"], hub.head(), "{status}");
    assert_eq!(status["caught_up"], true, "{status}");

    reader.approve(&hub.api_url(), WORKSPACE, &[REPO, "acme/private"], 2);
    reader.sync();

    let ids = reader.memory_ids();
    assert_eq!(
        ids.len(),
        1501,
        "every held memory now applied: {}",
        ids.len()
    );
    let status = reader.exchange_status();
    assert_eq!(status["pull"]["held"]["policy"], 0, "{status}");
}

// ---------------------------------------------------------------------------
// AC-7: a stalled pull replays safely; a dropped push response resends whole
// ---------------------------------------------------------------------------

#[test]
fn failed_import_stalls_then_replays_safely() {
    use comemory::config::Paths;
    use comemory::domains::memories::MemoryStore;

    let hub = Hub::start();
    let writer = approved_client(&hub);
    let bodies: Vec<String> = (1..=10)
        .map(|n| format!("{} (m{n})", guide_body(n)))
        .collect();
    let ids: Vec<String> = bodies.iter().map(|b| writer.save(b, REPO)).collect();
    writer.sync();

    let reader = Client::new();
    reader.login(&hub);
    reader.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);

    let blocked_path = MemoryStore::new(Paths::new(reader.data_dir())).planned_path(&bodies[5]);
    std::fs::create_dir_all(&blocked_path).expect("block m6's write target with a directory");

    let (_, stdout, _) = reader.cli_raw(&["sync"], &[]);
    let run = parse_json(&stdout);

    let applied = reader.memory_ids();
    for id in &ids[..5] {
        assert!(applied.contains(id), "m1..m5 applied: {applied:?}");
    }
    for id in &ids[5..] {
        assert!(
            !applied.contains(id),
            "m6..m10 must not apply while the write is blocked: {applied:?}"
        );
    }

    let (_, seq_m6) = hub_feed_row(&hub, &ids[5]);
    let (_, seq_m5) = hub_feed_row(&hub, &ids[4]);
    let status = reader.exchange_status();
    assert_eq!(status["pull"]["stalled_at"], seq_m6, "{status}");
    assert!(!status["pull"]["stall_reason"].is_null(), "{status}");
    // Every entry below m6 applied — m5 and the runs the writer shared
    // around it — so the cursor names the position right before the stall.
    assert!(seq_m5 < seq_m6);
    assert_eq!(
        applied_sequence(&reader),
        seq_m6 - 1,
        "the cursor names the last applied entry"
    );
    assert_eq!(run["exchange"]["end"], "stalled", "{run}");

    std::fs::remove_dir(&blocked_path).expect("unblock m6's write target");
    reader.sync();

    let applied = reader.memory_ids();
    for id in &ids {
        assert!(
            applied.contains(id),
            "all ten applied after the replay: {applied:?}"
        );
    }
    for id in &ids {
        let (op_id, _) = hub_feed_row(&hub, id);
        assert_eq!(
            receipt_count(&reader, &op_id),
            1,
            "exactly one receipt for {id}"
        );
        assert_eq!(
            synced_feed_rows(&reader, id),
            1,
            "exactly one synced feed row for {id}"
        );
        assert_eq!(
            markdown_files(&reader, id),
            1,
            "exactly one markdown file for {id}"
        );
    }
}

#[test]
fn a_resent_mixed_batch_gets_its_original_answers() {
    let hub = Hub::start();
    let a = approved_client(&hub);
    let m = a.save(
        "a memory the hub will delete out from under a pending patch",
        REPO,
    );
    a.sync();

    let b = Client::new();
    b.login(&hub);
    b.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    b.sync();
    assert!(
        b.memory_ids().contains(&m),
        "B pulled M before the hub deletes it: {:?}",
        b.memory_ids()
    );

    hub.engine().cli(&["delete", &m]);

    // The two operations must reach the upstream in one manual batch, not
    // one by one through the inline push after the save.
    std::fs::write(
        a.data_dir().join("config.toml"),
        "[sync]\npush_on_save = false\n",
    )
    .expect("config");
    let a_serve = a.serve();
    let (status, body) = a_serve.patch(
        &format!("/api/v1/memories/{m}"),
        &json!({"tags": ["stale-patch"]}),
    );
    assert!(status < 300, "A's patch of M: {status} {body}");
    drop(a_serve);
    let n = a.save(
        "a fresh memory queued in the same batch as the stale patch",
        REPO,
    );

    let pending: Vec<String> = a
        .outbox()
        .into_iter()
        .filter(|(_, state, _)| state == "pending")
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(
        pending.len(),
        2,
        "the patch and the save are both queued: {pending:?}"
    );
    let (op_stale, op_ok) = (pending[0].clone(), pending[1].clone());

    let before_import = hub.proxy.log().len();
    hub.proxy.arm(Fault::DropResponse {
        path: "/sync/replica/import".into(),
        times: 1,
    });
    let _ = a.cli_raw(&["sync"], &[]);
    let _ = a.cli_raw(&["sync"], &[]);

    let feed_hits = hub
        .feed()
        .into_iter()
        .filter(|(op, _, _)| op == &op_ok)
        .count();
    assert_eq!(feed_hits, 1, "op_ok appears exactly once in the hub feed");

    let op_ok_upstream_sequence: Option<i64> = a
        .open()
        .query_row(
            "SELECT upstream_sequence FROM replica_operation WHERE operation_id = ?1 AND state = 'accepted'",
            [&op_ok],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("op_ok not accepted: {e}"));
    let (feed_op_for_n, hub_seq_for_n) = hub_feed_row(&hub, &n);
    assert_eq!(
        feed_op_for_n, op_ok,
        "the feed's operation for N is the one A queued"
    );
    assert_eq!(
        op_ok_upstream_sequence,
        Some(hub_seq_for_n),
        "op_ok's upstream sequence matches the hub feed"
    );

    let (op_stale_state, op_stale_disposition): (String, Option<String>) = a
        .open()
        .query_row(
            "SELECT state, disposition FROM replica_operation WHERE operation_id = ?1",
            [&op_stale],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or_else(|e| panic!("op_stale row missing: {e}"));
    assert_eq!(
        op_stale_state, "rejected",
        "op_stale is rejected: {op_stale_disposition:?}"
    );
    assert_eq!(op_stale_disposition.as_deref(), Some("rejected_stale"));

    let full_log = hub.proxy.log();
    let import_requests: Vec<_> = full_log[before_import..]
        .iter()
        .filter(|l| l.path.contains("/sync/replica/import"))
        .collect();
    assert_eq!(
        import_requests.len(),
        2,
        "the batch was sent, dropped, then resent: {import_requests:?}"
    );
    for request in &import_requests {
        assert!(
            request.body.contains(&op_ok),
            "both import bodies carry op_ok's id: {}",
            request.body
        );
    }
}

// ---------------------------------------------------------------------------
// AC-8: an entry of a kind/version this build cannot read stalls the pull
// ---------------------------------------------------------------------------

#[test]
fn unknown_kind_stalls_the_cursor() {
    use comemory::store::connection;
    use comemory::store::replica_journal::{
        NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin, append, stream_epoch,
    };
    use comemory::utilities::canonical_json::bytes_and_digest;

    let hub = Hub::start();
    let writer = approved_client(&hub);
    let first_three: Vec<String> = (1..=3)
        .map(|n| writer.save(&format!("{} (early #{n})", guide_body(n)), REPO))
        .collect();
    writer.sync();

    {
        let mut conn = connection::open(hub.data_dir().join("comemory.db")).expect("open hub db");
        let epoch = stream_epoch(&conn).expect("hub epoch");
        let (bytes, digest) = bytes_and_digest(&json!({
            "kind": "future_kind",
            "body": "a newer engine's own entity, unreadable by this build",
        }))
        .expect("canonical payload");
        let bytes = std::str::from_utf8(&bytes)
            .expect("utf8 canonical bytes")
            .to_string();
        let tx = conn.transaction().expect("hub tx");
        append(
            &tx,
            &epoch,
            &NewOperation {
                operation_id: "op-20260924-futurekind00000000000000000000001",
                entity_kind: "future_kind",
                entity_key: "fk-1",
                op: ReplicaOp::Upsert,
                payload: Some(PayloadRef {
                    digest: &digest,
                    bytes: &bytes,
                }),
                schema_version: 1,
                repository: Some(REPO),
                origin: ReplicaOrigin::Local,
                at: "2026-09-24T10:00:00Z",
            },
        )
        .expect("append the newer-engine row");
        tx.commit().expect("commit hub tx");
    }

    let last_two: Vec<String> = (4..=5)
        .map(|n| writer.save(&format!("{} (late #{n})", guide_body(n)), REPO))
        .collect();
    writer.sync();

    let reader = Client::new();
    reader.login(&hub);
    reader.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    let l = reader.save("B's own local memory, pushed despite the stall", REPO);
    let (_, stdout, _) = reader.cli_raw(&["sync"], &[]);
    let run = parse_json(&stdout);

    let applied = reader.memory_ids();
    for id in &first_three {
        assert!(applied.contains(id), "W's first three applied: {applied:?}");
    }
    for id in &last_two {
        assert!(
            !applied.contains(id),
            "nothing after the unreadable row applies: {applied:?}"
        );
    }

    let (_, seq_fk) = hub_feed_row(&hub, "fk-1");
    let status = reader.exchange_status();
    assert_eq!(
        status["pull"]["stall_reason"], "incompatible_version",
        "{status}"
    );
    assert_eq!(status["pull"]["stalled_at"], seq_fk, "{status}");
    assert_eq!(run["exchange"]["end"], "stalled", "{run}");
    assert_eq!(run["exchange"]["more"], false, "{run}");

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
        hub_ids.contains(&l),
        "L still reached the hub despite the stalled pull: {hub_ids:?}"
    );
}
