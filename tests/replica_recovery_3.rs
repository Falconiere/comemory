#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! GC reach over real replica state (#256, B-6, AC-6): a real `comemory gc`
//! run against a real spawned engine, its journal, outbox and staged parts.
//! And permanent erase (#256, B-5, AC-7/AC-8) through a real hub, real
//! clients syncing over real HTTP, and every file the erasing engine keeps.

#[path = "common/replica_support.rs"]
mod replica_support;

#[path = "common/replica_events_support.rs"]
mod replica_events_support;

#[path = "common/exchange_support.rs"]
mod exchange_support;

#[path = "common/fault_proxy.rs"]
mod fault_proxy;

use std::path::{Path, PathBuf};
use std::time::Duration;

use comemory::store::replica_replay::{self, ReplayRow};
use comemory::store::replica_staging;
use comemory::store::sync_exchange::ExchangeKey;
use exchange_support::{Client, Hub, REPO, WORKSPACE};
use replica_events_support::{age, feedback, journal, pair};
use replica_support::{active_generation, feed_ops, mark_all_pushed, publish_locally};
use serde_json::{Value, json};

#[test]
fn gc_keeps_what_is_owed_and_expires_the_rest() {
    let p = pair();
    let active = publish_locally(&p.a.data_dir(), "checkout-a");

    // One verdict settles (an ordinary push acknowledgment) before the next
    // is recorded, so only its own outbox row is marked accepted; the second
    // stays pending — an upload this client still owes an unreachable hub.
    feedback(&p.a, Some("delivered"), &p.scene, false);
    let delivered_digest = journal(&p.a.data_dir(), "feedback_event")[0]["digest"]
        .as_str()
        .expect("digest")
        .to_string();
    mark_all_pushed(&p.a.data_dir());
    feedback(&p.a, Some("owed"), &p.scene, false);
    let entries = journal(&p.a.data_dir(), "feedback_event");
    assert_eq!(entries.len(), 2);
    let owed_entry = entries
        .iter()
        .find(|e| e["digest"] != json!(delivered_digest))
        .expect("the second event");
    let owed_digest = owed_entry["digest"].as_str().expect("digest").to_string();

    // Feedback verdicts are not themselves durably owed through the outbox
    // the way a memory upsert is — this stands in for the exchange gate
    // (#256, B-3) offering it once that lands, so the digest is genuinely
    // named by a `pending` row when retention runs.
    comemory::store::replica_outbox::enqueue(
        &p.a.db(),
        &comemory::store::replica_journal::NewOperation {
            operation_id: owed_entry["operation_id"].as_str().expect("operation_id"),
            entity_kind: "feedback_event",
            entity_key: owed_entry["entity_key"].as_str().expect("entity_key"),
            op: comemory::store::replica_journal::ReplicaOp::Upsert,
            payload: Some(comemory::store::replica_journal::PayloadRef {
                digest: &owed_digest,
                bytes: "unused: only the digest is read back",
            }),
            schema_version: 1,
            repository: Some("Falconiere/comemory"),
            origin: comemory::store::replica_journal::ReplicaOrigin::Local,
            at: "2026-09-25T10:00:00Z",
        },
        None,
    )
    .expect("enqueue the owed push");

    // A SIGKILLed pull's leftover scratch copy of the digest about to expire,
    // and a staged upload nobody ever finished — both must be gone once `gc`
    // reaches past retention, alongside the payload itself.
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_gc_test");
    replica_replay::offer(
        &p.a.db(),
        &key,
        &ReplayRow {
            entity_kind: "feedback_event".to_string(),
            entity_key: "scratch-leftover".to_string(),
            sequence: 1,
            entry_json: format!(r#"{{"payload_digest":"{delivered_digest}"}}"#),
        },
    )
    .expect("seed replay scratch");
    replica_staging::put_part(
        &p.a.db(),
        "stray-staging",
        0,
        1,
        "{}",
        "2020-01-01T00:00:00Z",
    )
    .expect("seed a stray staged part");

    age(&p.a.data_dir(), "feedback_events", "1 = 1");
    let feed_before = feed_ops(&p.a.data_dir());

    let response = p.a.cli(&["gc"]);
    assert_eq!(response["staged_rows"].as_u64(), Some(1), "{response}");

    let persisted_staged_rows: i64 =
        p.a.db()
            .query_row(
                "SELECT staged_rows FROM gc_runs ORDER BY at DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .expect("gc_runs row");
    assert_eq!(
        persisted_staged_rows, 1,
        "the sweep is persisted, not only reported"
    );

    let after = journal(&p.a.data_dir(), "feedback_event");
    let owed_after = after
        .iter()
        .find(|e| e["digest"] == json!(owed_digest))
        .expect("owed entry survives");
    assert!(
        owed_after["payload"].is_object(),
        "an owed upload keeps its bytes through gc: {owed_after}"
    );
    assert!(owed_after["redaction"].is_null());
    let delivered_after = after
        .iter()
        .find(|e| e["digest"] == json!(delivered_digest))
        .expect("delivered entry survives");
    assert!(
        delivered_after["payload"].is_null(),
        "a delivered, settled event expires: {delivered_after}"
    );
    assert_eq!(delivered_after["redaction"], json!("expired"));

    assert_eq!(
        replica_replay::next(&p.a.db(), &key, 10).expect("next"),
        Vec::<ReplayRow>::new(),
        "the killed replay's scratch copy of the expired digest is cleared"
    );
    let staged_left: i64 =
        p.a.db()
            .query_row("SELECT count(*) FROM replica_staged_part", [], |r| r.get(0))
            .expect("count");
    assert_eq!(staged_left, 0, "the abandoned upload is swept");

    assert_eq!(
        feed_ops(&p.a.data_dir()),
        feed_before,
        "no feed row is removed by either sweep"
    );
    assert_eq!(
        active_generation(&p.a.data_dir(), "checkout-a"),
        Some(active),
        "the active generation is untouched"
    );
}

// ---------------------------------------------------------------------------
// Permanent erase (#256, B-5).
// ---------------------------------------------------------------------------

/// A client logged into `hub` with `REPO` approved.
fn approved_client(hub: &Hub) -> Client {
    let client = Client::new();
    client.login(hub);
    client.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    client
}

/// Every file under `dir` whose bytes contain `needle`, skipping `except`
/// (the rollback snapshots an erase names).
fn files_holding(dir: &Path, needle: &str, except: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return found,
        Err(e) => panic!("cannot list {}: {e}", dir.display()),
    };
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if except.contains(&path) {
            continue;
        }
        if path.is_dir() {
            found.extend(files_holding(&path, needle, except));
        } else if match std::fs::read(&path) {
            Ok(bytes) => bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
            // A sidecar a live engine removed after the listing holds nothing.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => panic!("cannot read {}: {e}", path.display()),
        } {
            found.push(path);
        }
    }
    found
}

/// `comemory --json erase <target> --confirm` over `data_dir`, asserting
/// success — for the hub, whose data directory the CLI shares with `serve`.
fn erase_over(data_dir: &Path, target: &[&str]) -> Value {
    let mut args = vec!["erase"];
    args.extend_from_slice(target);
    args.push("--confirm");
    let (code, stdout, stderr) = replica_support::cli_raw(data_dir, &args);
    assert_eq!(code, 0, "erase failed: {stderr}");
    serde_json::from_str(stdout.trim()).expect("erase report")
}

fn snapshots(report: &Value) -> Vec<PathBuf> {
    report["snapshots_with_prior_state"]
        .as_array()
        .expect("snapshots")
        .iter()
        .map(|p| PathBuf::from(p.as_str().expect("path")))
        .collect()
}

/// The import envelope `client` sent for its first upsert of `key`: the
/// same operation id, digest and bytes, read back from its own journal.
fn original_envelope(conn: &rusqlite::Connection, kind: &str, key: &str) -> Value {
    let (operation_id, digest, bytes, repository): (String, String, String, Option<String>) = conn
        .query_row(
            "SELECT o.operation_id, o.payload_digest, p.bytes, o.repository \
               FROM replica_operation o JOIN replica_payload p ON p.digest = o.payload_digest \
              WHERE o.entity_kind = ?1 AND o.entity_key = ?2 AND o.op = 'upsert' \
              ORDER BY o.created_at, o.rowid LIMIT 1",
            [kind, key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("the client's own upload");
    let payload: Value = serde_json::from_str(&bytes).expect("payload json");
    json!({
        "protocol": "replica-v1",
        "operations": [{
            "operation_id": operation_id,
            "entity_kind": kind,
            "entity_key": key,
            "op": "upsert",
            "schema_version": 1,
            "payload_digest": digest,
            "payload": payload,
            "repository": repository,
        }]
    })
}

fn dispositions_of(response: &Value) -> Vec<String> {
    response["data"]["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|r| r["disposition"].as_str().expect("disposition").to_string())
        .collect()
}

/// `(op, state, disposition)` of every outbox row for `key`, oldest first.
fn outbox_rows(conn: &rusqlite::Connection, key: &str) -> Vec<(String, String, Option<String>)> {
    let mut statement = conn
        .prepare(
            "SELECT op, state, disposition FROM replica_operation \
              WHERE entity_key = ?1 ORDER BY created_at, rowid",
        )
        .expect("prepare");
    statement
        .query_map([key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

fn scalar(conn: &rusqlite::Connection, sql: &str, key: &str) -> i64 {
    conn.query_row(sql, [key], |r| r.get(0)).expect("scalar")
}

/// Row count of `table`.
fn rows(conn: &rusqlite::Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .expect("count")
}

/// Move the pull cursor back to the start of the stream, so the next sync
/// pulls the whole feed again.
fn rewind_cursor(client: &Client) {
    client
        .open()
        .execute(
            "UPDATE replica_cursor SET applied_sequence = 0, anchor_sequence = NULL, \
             anchor_operation_id = NULL",
            [],
        )
        .expect("rewind the cursor");
}

/// Restore `id` from `client`'s trash through its own serve, then sync.
fn restore_and_sync(client: &Client, id: &str) {
    let serve = client.serve();
    let (status, body) = serve.post(&format!("/api/v1/trash/{id}/restore"), &json!({}));
    assert!(status < 300, "restore {id}: {status} {body}");
    drop(serve);
    client.sync();
}

/// A word nothing ever queries: finding it after the erase means a copy of
/// the memory's text survived.
const TOKEN: &str = "vexillumquartzine7719";

#[test]
fn erase_leaves_only_the_barrier() {
    let hub = Hub::start();
    let a = approved_client(&hub);
    let b = approved_client(&hub);
    let id = a.save(
        &format!(
            "Staged uploads keep their parts for a day before the sweep reclaims them. \
             The marker for this decision is {TOKEN}."
        ),
        REPO,
    );
    let found = a.cli(&[
        "find",
        "staged uploads sweep reclaims parts",
        "--repo",
        REPO,
    ]);
    assert!(
        found["hits"]
            .as_array()
            .expect("hits")
            .iter()
            .any(|h| h["id"] == json!(id)),
        "found by other words: {found}"
    );
    let query_id = found["query_id"].as_str().expect("query id").to_string();
    a.cli(&["feedback", &query_id, "--used", &id]);
    let restored_later = a.save(
        "A second memory the hub deletes and restores the ordinary way.",
        REPO,
    );
    let purged_later = a.save(
        "A third memory the hub trashes and gc purges, still restorable.",
        REPO,
    );
    a.sync();
    // The hub's own edit: an upload the hub owes nobody, still pending when
    // the erase runs, so the erase has one to withdraw.
    let (status, body) = hub.engine().patch(
        &format!("/api/v1/memories/{id}"),
        &json!({"tags": ["erase", "hub-edit"]}),
    );
    assert!(status < 300, "hub edit: {status} {body}");
    b.sync();
    assert!(b.memory_ids().contains(&id), "the peer holds the memory");
    let envelope = original_envelope(&a.open(), "memory", &id);
    let hub_db = hub.db();
    let feed_before: Vec<String> = hub
        .feed()
        .into_iter()
        .filter(|(_, key, _)| key == &id)
        .map(|(_, _, op)| op)
        .collect();
    assert_eq!(
        feed_before,
        vec!["upsert", "upsert"],
        "A's upload and the hub's edit"
    );
    assert!(
        !files_holding(&hub.data_dir(), TOKEN, &[]).is_empty(),
        "the premise: the hub holds the text"
    );

    let report = erase_over(&hub.data_dir(), &["--memory", &id]);

    assert_eq!(report["tombstoned"], json!(true), "{report}");
    assert_eq!(report["wal_truncated"], json!(true), "{report}");
    assert_eq!(report["operations_withdrawn"], json!(1), "{report}");
    assert!(
        report["payloads_erased"].as_u64().expect("count") >= 2,
        "{report}"
    );
    let named = snapshots(&report);
    assert_eq!(
        files_holding(&hub.data_dir(), TOKEN, &named),
        Vec::<PathBuf>::new(),
        "the token survives outside the named snapshots {named:?}"
    );
    let feed_after: Vec<String> = hub
        .feed()
        .into_iter()
        .filter(|(_, key, _)| key == &id)
        .map(|(_, _, op)| op)
        .collect();
    assert_eq!(
        feed_after,
        vec!["upsert", "upsert", "tombstone"],
        "feed rows kept"
    );
    let original_id = envelope["operations"][0]["operation_id"]
        .as_str()
        .expect("op id");
    assert_eq!(
        scalar(
            &hub_db,
            "SELECT COUNT(*) FROM replica_receipt WHERE operation_id = ?1 AND disposition = 'accepted'",
            original_id
        ),
        1,
        "the receipt is kept"
    );
    assert_eq!(
        scalar(
            &hub_db,
            "SELECT deleted FROM replica_revision WHERE entity_key = ?1",
            &id
        ),
        1,
        "the revision is kept, tombstoned"
    );
    assert_eq!(
        outbox_rows(&hub_db, &id),
        vec![
            (
                "upsert".into(),
                "rejected".into(),
                Some("payload_erased".into())
            ),
            ("tombstone".into(), "pending".into(), None),
        ],
        "the hub's own pending upload is withdrawn; the tombstone still goes"
    );

    // The barrier: a replay of the original import, and a peer's restore
    // carrying the deleted revision's bytes, are both `payload_erased`.
    let (status, replay) = hub.engine().post("/api/v1/sync/replica/import", &envelope);
    assert_eq!(status, 200, "{replay}");
    assert_eq!(dispositions_of(&replay), vec!["payload_erased"]);
    b.sync();
    assert!(!b.memory_ids().contains(&id), "the peer took the tombstone");
    restore_and_sync(&b, &id);
    let restore = outbox_rows(&b.open(), &id)
        .into_iter()
        .rfind(|(op, _, _)| op == "restore")
        .expect("the peer journalled a restore");
    assert_eq!(
        restore,
        (
            "restore".into(),
            "rejected".into(),
            Some("payload_erased".into())
        )
    );
    assert!(
        !hub.feed()
            .iter()
            .any(|(_, key, op)| key == &id && op == "restore"),
        "the hub materialized nothing"
    );

    // A peer pulling the whole feed again applies nothing.
    let b_db = b.open();
    let (b_feed, b_receipts) = (rows(&b_db, "replica_feed"), rows(&b_db, "replica_receipt"));
    let b_live = b.memory_ids();
    rewind_cursor(&b);
    b.sync();
    assert_eq!(rows(&b_db, "replica_feed"), b_feed, "no position applied");
    assert_eq!(
        rows(&b_db, "replica_receipt"),
        b_receipts,
        "no acceptance recorded"
    );
    assert_eq!(b.memory_ids(), b_live);

    // An ordinary delete and restore on the hub still works.
    let deleted = reqwest::blocking::Client::new()
        .delete(format!(
            "{}/api/v1/memories/{restored_later}?confirm=true",
            hub.engine().base
        ))
        .bearer_auth(&hub.engine().token)
        .send()
        .expect("delete");
    assert!(deleted.status().is_success(), "{}", deleted.status());
    let (status, body) = hub.engine().post(
        &format!("/api/v1/trash/{restored_later}/restore"),
        &json!({}),
    );
    assert!(status < 300, "restore: {status} {body}");
    assert_eq!(
        scalar(
            &hub_db,
            "SELECT COUNT(*) FROM memories WHERE id = ?1 AND deleted_at IS NULL",
            &restored_later
        ),
        1,
        "the ordinary restore is live"
    );

    // A trashed memory gc purges on the hub stays restorable from a peer.
    let deleted = reqwest::blocking::Client::new()
        .delete(format!(
            "{}/api/v1/memories/{purged_later}?confirm=true",
            hub.engine().base
        ))
        .bearer_auth(&hub.engine().token)
        .send()
        .expect("delete");
    assert!(deleted.status().is_success(), "{}", deleted.status());
    for entry in std::fs::read_dir(hub.data_dir().join("memories/.trash"))
        .expect("trash")
        .flatten()
    {
        std::fs::OpenOptions::new()
            .write(true)
            .open(entry.path())
            .expect("open trash file")
            .set_modified(std::time::SystemTime::now() - Duration::from_hours(90 * 24))
            .expect("age trash file");
    }
    hub_db
        .execute(
            "UPDATE memories SET deleted_at = '2020-01-01T00:00:00Z' WHERE id = ?1",
            [&purged_later],
        )
        .expect("age deletion");
    let (code, _, stderr) = replica_support::cli_raw(&hub.data_dir(), &["gc"]);
    assert_eq!(code, 0, "gc: {stderr}");
    assert_eq!(
        scalar(
            &hub_db,
            "SELECT COUNT(*) FROM memories WHERE id = ?1",
            &purged_later
        ),
        0,
        "gc purged the rows"
    );
    b.sync();
    assert!(
        !b.memory_ids().contains(&purged_later),
        "the peer took the deletion"
    );
    restore_and_sync(&b, &purged_later);
    assert_eq!(
        outbox_rows(&b.open(), &purged_later)
            .into_iter()
            .rfind(|(op, _, _)| op == "restore")
            .map(|(_, state, _)| state),
        Some("accepted".to_string()),
        "a gc purge is housekeeping: the peer's restore is accepted"
    );
    assert_eq!(
        scalar(
            &hub_db,
            "SELECT COUNT(*) FROM memories WHERE id = ?1 AND deleted_at IS NULL",
            &purged_later
        ),
        1,
        "and the hub holds it live again"
    );
    assert_eq!(
        files_holding(&hub.data_dir(), TOKEN, &named),
        Vec::<PathBuf>::new(),
        "no offer of the erased bytes stored them again"
    );
}

/// The guide the document case erases.
const ERASED_GUIDE: &str = "byo-vectors.md";

/// Up to five short plain-text lines of `document_id`'s chunks that no other
/// local document contains — probes whose absence proves the text is gone
/// from every table and journal copy, where JSON escaping cannot hide it.
fn probes_of(conn: &rusqlite::Connection, document_id: &str) -> Vec<String> {
    let mut statement = conn
        .prepare("SELECT text FROM document_chunks WHERE document_id = ?1 ORDER BY ordinal")
        .expect("prepare");
    let texts: Vec<String> = statement
        .query_map([document_id], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");
    let mut probes = Vec::new();
    for line in texts.iter().flat_map(|t| t.lines()) {
        let line = line.trim();
        let plain = line
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " ,.".contains(c));
        if line.len() < 30 || !plain {
            continue;
        }
        let probe: String = line.chars().take(40).collect();
        let elsewhere: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM document_chunks WHERE document_id != ?1 AND instr(text, ?2) > 0",
                [document_id, probe.as_str()],
                |r| r.get(0),
            )
            .expect("unique");
        if elsewhere == 0 && !probes.contains(&probe) {
            probes.push(probe);
        }
        if probes.len() == 5 {
            break;
        }
    }
    assert!(!probes.is_empty(), "the guide has plain lines to probe for");
    probes
}

/// Every `table.column` holding `probe` in `conn` — each table a document's
/// text can be copied into, and each journal copy of it.
fn tables_holding(conn: &rusqlite::Connection, probe: &str) -> Vec<&'static str> {
    [
        ("document_chunks", "text"),
        ("document_fts", "passage"),
        ("remote_document_chunk", "text"),
        ("remote_document_fts", "passage"),
        ("replica_payload", "bytes"),
        ("replica_replay", "entry_json"),
        ("replica_staged_part", "bytes"),
        ("candidate_observations", "text"),
    ]
    .into_iter()
    .filter(|(table, column)| {
        conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE instr({column}, ?1) > 0"),
            [probe],
            |r| r.get::<_, i64>(0),
        )
        .expect("probe")
            > 0
    })
    .map(|(table, _)| table)
    .collect()
}

/// `(op, digest)` of every position `client` journalled for `shared_id`.
fn document_positions(client: &Client, shared_id: &str) -> Vec<(String, Option<String>)> {
    let conn = client.open();
    let mut statement = conn
        .prepare(
            "SELECT op, payload_digest FROM replica_feed \
              WHERE entity_kind = 'document_revision' AND entity_key = ?1 ORDER BY sequence",
        )
        .expect("prepare");
    statement
        .query_map([shared_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}

fn assert_gone(client: &Client, probes: &[String], report: &Value, side: &str) {
    let conn = client.open();
    let named = snapshots(report);
    for probe in probes {
        assert_eq!(
            tables_holding(&conn, probe),
            Vec::<&str>::new(),
            "{side} still holds {probe:?}"
        );
        assert_eq!(
            files_holding(&client.data_dir(), probe, &named),
            Vec::<PathBuf>::new(),
            "{side}'s files still hold {probe:?}"
        );
    }
}

#[test]
fn erased_documents_stay_erased() {
    let hub = Hub::start();
    let a = approved_client(&hub);
    let b = approved_client(&hub);
    let work = tempfile::tempdir().expect("workdir");
    // Documents are shared only from under the label's indexed root, so the
    // guides live inside a real indexed checkout.
    let checkout = replica_support::pinned_repo(work.path(), 3);
    a.cli(&[
        "index-code",
        "--repo",
        REPO,
        "--path",
        checkout.to_str().expect("utf8"),
    ]);
    let guides = replica_support::docs_tree(work.path(), "pinned-repo").join("docs/guides");
    a.cli(&["index", guides.to_str().expect("utf8"), "--repo", REPO]);
    a.sync();
    b.sync();
    let (document_id, shared_id): (String, String) = a
        .open()
        .query_row(
            "SELECT document_id, shared_id FROM document_share WHERE path = ?1",
            [format!("docs/guides/{ERASED_GUIDE}")],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the guide is shared");
    let probes = probes_of(&a.open(), &document_id);
    for probe in &probes {
        assert!(
            !tables_holding(&a.open(), probe).is_empty(),
            "premise on the origin"
        );
        assert!(
            !tables_holding(&b.open(), probe).is_empty(),
            "premise on the peer"
        );
    }
    let envelope = original_envelope(&a.open(), "document_revision", &shared_id);
    let source = guides.join(ERASED_GUIDE);
    let original_bytes = std::fs::read(&source).expect("source");

    let on_origin = a.cli(&["erase", "--document", &shared_id, "--confirm"]);
    let on_peer = b.cli(&["erase", "--document", &shared_id, "--confirm"]);

    assert_eq!(
        on_origin["tombstoned"],
        json!(true),
        "the origin shares it: {on_origin}"
    );
    assert_eq!(
        on_peer["tombstoned"],
        json!(false),
        "a pulled copy is local only: {on_peer}"
    );
    for report in [&on_origin, &on_peer] {
        assert!(
            report["payloads_erased"].as_u64().expect("count") >= 1,
            "{report}"
        );
        assert_eq!(report["wal_truncated"], json!(true), "{report}");
    }
    assert_gone(&a, &probes, &on_origin, "the origin");
    assert_gone(&b, &probes, &on_peer, "the peer");
    assert_eq!(
        std::fs::read(&source).expect("source"),
        original_bytes,
        "the origin's source file is untouched"
    );

    // A later pull, and a replay of the erased revision on either side,
    // apply nothing.
    let peer_feed = rows(&b.open(), "replica_feed");
    rewind_cursor(&b);
    b.sync();
    assert_eq!(
        rows(&b.open(), "replica_feed"),
        peer_feed,
        "the re-pull applied nothing"
    );
    for (client, side) in [(&a, "origin"), (&b, "peer")] {
        let serve = client.serve();
        let (status, replay) = serve.post("/api/v1/sync/replica/import", &envelope);
        drop(serve);
        assert_eq!(status, 200, "{replay}");
        assert_eq!(
            dispositions_of(&replay),
            vec!["payload_erased"],
            "on the {side}"
        );
    }
    assert_gone(&a, &probes, &on_origin, "the origin after the replay");
    assert_gone(
        &b,
        &probes,
        &on_peer,
        "the peer after the re-pull and replay",
    );

    // An unchanged re-index records the share blocked `erased` and journals
    // nothing; an edit is new bytes and shares a new revision.
    let before_reindex = document_positions(&a, &shared_id);
    a.cli(&["index", guides.to_str().expect("utf8"), "--repo", REPO]);
    let blocked: Option<String> = a
        .open()
        .query_row(
            "SELECT blocked_reason FROM document_share WHERE shared_id = ?1",
            [&shared_id],
            |r| r.get(0),
        )
        .expect("share row");
    assert_eq!(blocked.as_deref(), Some("erased"));
    assert_eq!(
        document_positions(&a, &shared_id),
        before_reindex,
        "nothing journalled"
    );
    assert_eq!(
        before_reindex
            .iter()
            .map(|(op, _)| op.as_str())
            .collect::<Vec<_>>(),
        vec!["upsert", "tombstone"]
    );

    let mut edited = original_bytes;
    edited.extend_from_slice(b"\n\n## After the erase\n\nThis paragraph was written later.\n");
    std::fs::write(&source, edited).expect("edit the guide");
    a.cli(&["index", guides.to_str().expect("utf8"), "--repo", REPO]);
    let positions = document_positions(&a, &shared_id);
    let (op, digest) = positions.last().expect("a position").clone();
    assert_eq!(positions.len(), 3, "one new revision");
    assert_eq!(op, "upsert");
    let digest = digest.expect("the revision names a payload");
    let bytes: Option<String> = a
        .open()
        .query_row(
            "SELECT bytes FROM replica_payload WHERE digest = ?1",
            [&digest],
            |r| r.get(0),
        )
        .expect("payload row");
    assert!(bytes.is_some(), "the new revision keeps its bytes");
    let shared: Option<String> = a
        .open()
        .query_row(
            "SELECT blocked_reason FROM document_share WHERE shared_id = ?1",
            [&shared_id],
            |r| r.get(0),
        )
        .expect("share row");
    assert_eq!(shared, None, "shared again");
    assert!(
        a.outbox()
            .iter()
            .any(|(operation_id, state, _)| state == "pending"
                && positions_of_operation(&a, operation_id) == Some(digest.clone())),
        "and queued for upload"
    );
}

/// The payload digest `client`'s outbox row `operation_id` names.
fn positions_of_operation(client: &Client, operation_id: &str) -> Option<String> {
    client
        .open()
        .query_row(
            "SELECT payload_digest FROM replica_operation WHERE operation_id = ?1",
            [operation_id],
            |r| r.get(0),
        )
        .expect("operation")
}
