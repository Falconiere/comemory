#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Backup and restore of a real hub (#256, B-4, AC-9): `comemory backup
//! create`, writes, an erase, and `comemory backup restore` — killed with a
//! real `SIGKILL` mid-swap and rerun — against a spawned `comemory serve`,
//! with a real client syncing over real HTTP through the fault proxy. And
//! the local-only restore whose manifest is gone, refusing every sync route
//! through a `comemory rebuild` until `backup merge-erasures`.

#[path = "common/replica_support.rs"]
mod replica_support;

#[path = "common/exchange_support.rs"]
mod exchange_support;

#[path = "common/fault_proxy.rs"]
mod fault_proxy;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin;
use exchange_support::{Client, Hub, REPO, WORKSPACE};
use serde_json::{Value, json};

/// A word nothing ever queries: finding it after the restore means erased
/// content came back.
const TOKEN: &str = "vexillumRestored4401";

/// A client logged into `hub` with `REPO` approved.
fn approved_client(hub: &Hub) -> Client {
    let client = Client::new();
    client.login(hub);
    client.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    client
}

/// `comemory --json <args>` over the hub's data directory, asserting success.
fn hub_cli(hub: &Hub, args: &[&str]) -> Value {
    let (code, stdout, stderr) = replica_support::cli_raw(&hub.data_dir(), args);
    assert_eq!(code, 0, "comemory {args:?} failed: {stderr}");
    serde_json::from_str(stdout.trim()).unwrap_or(Value::Null)
}

/// Save a memory on `client` without the inline push: an edit it still owes.
fn save_pending(client: &Client, body: &str) -> String {
    let (code, stdout, stderr) = client.cli_raw(
        &["save", "--kind", "decision", "--repo", REPO, body],
        &[("COMEMORY_SYNC_PUSH_ON_SAVE", "0")],
    );
    assert_eq!(code, 0, "save: {stderr}");
    let saved: Value = serde_json::from_str(&stdout).expect("save json");
    saved["id"].as_str().expect("id").to_string()
}

/// Every file under `dir` whose bytes contain `needle`, skipping `except`.
fn files_holding(dir: &Path, needle: &str, except: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if except.contains(&path) {
            continue;
        }
        if path.is_dir() {
            found.extend(files_holding(&path, needle, except));
        } else if std::fs::read(&path)
            .unwrap_or_default()
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
        {
            found.push(path);
        }
    }
    found
}

/// The paths a restore report names as kept.
fn kept(report: &Value) -> Vec<PathBuf> {
    report["pre_restore"]
        .as_array()
        .expect("pre_restore")
        .iter()
        .map(|p| PathBuf::from(p.as_str().expect("path")))
        .collect()
}

/// `(epoch, device id)` the hub's database carries.
fn identity_of(hub: &Hub) -> (String, String) {
    let conn = hub.db();
    let epoch = conn
        .query_row("SELECT epoch FROM replica_stream", [], |r| r.get(0))
        .expect("epoch");
    let device = conn
        .query_row("SELECT device_id FROM replica_device", [], |r| r.get(0))
        .expect("device");
    (epoch, device)
}

/// Live memory ids in the hub's mirror.
fn hub_memory_ids(hub: &Hub) -> Vec<String> {
    let conn = hub.db();
    let mut statement = conn
        .prepare("SELECT id FROM memories WHERE deleted_at IS NULL ORDER BY id")
        .expect("prepare");
    statement
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}

/// Assert `(status, body)` is the `503 restore_unverified` refusal.
fn assert_refused(what: &str, (status, body): (u16, Value)) {
    assert_eq!(status, 503, "{what}: {body}");
    assert_eq!(
        body["error"]["code"],
        json!("restore_unverified"),
        "{what}: {body}"
    );
}

/// Every replica and legacy sync route, read and write, against `hub`.
fn every_sync_route(hub: &Hub) -> Vec<(String, (u16, Value))> {
    let engine = hub.engine();
    let tombstone = json!({"operation_id": "op-20260925-eeeeeeee", "entity_kind": "memory",
        "entity_key": "a1b2c3d4", "op": "tombstone", "schema_version": 1,
        "payload_digest": null, "payload": null, "observed_sequence": null,
        "repository": null, "vector": null});
    let mut answers = Vec::new();
    for path in [
        "/api/v1/sync/replica/changes?since=0",
        "/api/v1/sync/replica/manifest",
        "/api/v1/sync/replica/events?since=0",
        "/api/v1/sync/changes?since=0",
        "/api/v1/sync/manifest",
        "/api/v1/sync/code/manifest?repo=falconiere/comemory",
    ] {
        answers.push((format!("GET {path}"), engine.get(path)));
    }
    for (path, body) in [
        (
            "/api/v1/sync/replica/import",
            json!({"protocol": "replica-v1", "operations": []}),
        ),
        (
            "/api/v1/sync/replica/stage",
            json!({"protocol": "replica-v1", "staging_id": "s-1", "part_index": 0,
                   "part_count": 2, "bytes": "{"}),
        ),
        (
            "/api/v1/sync/replica/activate",
            json!({"protocol": "replica-v1", "staging_id": "s-2", "operation": tombstone}),
        ),
        ("/api/v1/sync/import", json!({"cursor": 0, "entries": []})),
        ("/api/v1/sync/code/import", json!({"repo": REPO})),
    ] {
        answers.push((format!("POST {path}"), engine.post(path, &body)));
    }
    answers
}

/// Wait until the restore running over `data` has moved the markdown and
/// is copying the database in place — where a held write lock parks it.
fn wait_for_swap(data: &Path) {
    let deadline = Instant::now() + Duration::from_mins(2);
    let pending = data.join("restore.pending");
    while Instant::now() < deadline {
        let swapping = std::fs::read(&pending)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|record| record["phase"] == "swapping");
        if swapping && !data.join("memories.restore").exists() {
            std::thread::sleep(Duration::from_millis(300));
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("the restore never reached its swap");
}

/// Run `backup restore <dir> --confirm` over the hub while a real writer
/// holds its database, SIGKILL it once it is mid-swap, and check the hub
/// refuses sync while the swap is pending.
fn restore_killed_mid_swap(hub: &Hub, dir: &Path) {
    let blocker = hub.db();
    blocker
        .execute_batch(
            "BEGIN IMMEDIATE; UPDATE schema_meta SET value = value WHERE key = 'version';",
        )
        .expect("hold the write lock");
    let mut child = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", hub.data_dir())
        .env("COMEMORY_SYNC_PAUSE_WAIT", "300s")
        .args(["--json", "backup", "restore"])
        .arg(dir)
        .arg("--confirm")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the restore");
    wait_for_swap(&hub.data_dir());
    assert_refused(
        "manifest mid-swap",
        hub.engine().get("/api/v1/sync/replica/manifest"),
    );
    child.kill().expect("SIGKILL mid-swap");
    let _ = child.wait();
    blocker.execute_batch("ROLLBACK;").expect("release");
    drop(blocker);
    assert!(hub.data_dir().join("restore.pending").exists());
    assert_refused(
        "changes after the kill",
        hub.engine().get("/api/v1/sync/replica/changes?since=0"),
    );
}

#[test]
fn restore_rotates_the_epoch_and_merges_erasures() {
    let mut hub = Hub::start();
    let a = approved_client(&hub);
    let kept_id = a.save("A decision the backup holds and the restore keeps.", REPO);
    let erased = a.save(
        &format!("A decision the hub later erases for good; its marker is {TOKEN}."),
        REPO,
    );
    a.sync();
    let backups = tempfile::tempdir().expect("backups");
    let dir = backups.path().join("hub");
    hub_cli(
        &hub,
        &["backup", "create", "--out", dir.to_str().expect("utf8")],
    );
    let backup_head = hub.head();
    a.save("A decision written after the backup.", REPO);
    a.sync();
    hub_cli(&hub, &["erase", "--memory", &erased, "--confirm"]);
    a.sync();
    let cursor = a.exchange_status()["applied_sequence"]
        .as_i64()
        .expect("cursor");
    assert!(
        cursor > backup_head + 1,
        "the premise: {cursor} > {backup_head}"
    );
    let owed = save_pending(
        &a,
        "An edit the client still owes when the hub is restored.",
    );
    let (epoch_before, device_before) = identity_of(&hub);

    restore_killed_mid_swap(&hub, &dir);
    let restored = hub_cli(
        &hub,
        &[
            "backup",
            "restore",
            dir.to_str().expect("utf8"),
            "--confirm",
        ],
    );

    assert_eq!(restored["resumed"], json!(true), "{restored}");
    assert_eq!(restored["restore_state"], Value::Null, "{restored}");
    let (epoch, device) = identity_of(&hub);
    assert_ne!(epoch, epoch_before, "a restore is a new stream");
    assert_eq!(device, device_before, "the device id is kept");
    let (status, manifest) = hub.engine().get("/api/v1/sync/replica/manifest");
    assert_eq!(status, 200, "sync answers once merged: {manifest}");
    assert_eq!(manifest["data"]["stream_epoch"], json!(epoch));
    assert!(hub_memory_ids(&hub).contains(&kept_id));
    assert!(
        !hub_memory_ids(&hub).contains(&erased),
        "the erased memory is not resurrected"
    );
    assert_eq!(
        files_holding(&hub.data_dir(), TOKEN, &kept(&restored)),
        Vec::<PathBuf>::new(),
        "no erased content outside what the restore replaced"
    );

    // The client's cursor is past the restored head, under the old epoch:
    // it rebootstraps, keeps and pushes its edit, and catches up.
    let synced = a.sync();
    assert_eq!(
        synced["exchange"]["rebootstrapped"],
        json!(true),
        "{synced}"
    );
    assert!(
        hub.feed().iter().any(|(_, key, _)| key == &owed),
        "the owed edit reached the restored hub"
    );
    assert!(!a.memory_ids().contains(&erased));
    let status = a.exchange_status();
    assert_eq!(status["caught_up"], json!(true), "{status}");

    // A `.bak` copied over `comemory.db` by hand: a new epoch and the merge
    // on the first sync read.
    let erased_digest: String = hub
        .db()
        .query_row(
            "SELECT payload_digest FROM replica_feed WHERE entity_key = ?1 \
             AND payload_digest IS NOT NULL ORDER BY sequence LIMIT 1",
            [&erased],
            |r| r.get(0),
        )
        .expect("the erased memory's digest");
    hub.stop();
    let db = hub.data_dir().join("comemory.db");
    for side in ["comemory.db-wal", "comemory.db-shm"] {
        let _ = std::fs::remove_file(hub.data_dir().join(side));
    }
    std::fs::copy(dir.join("comemory.db"), &db).expect("copy the backup over by hand");
    hub.restart();
    let (status, manifest) = hub.engine().get("/api/v1/sync/replica/manifest");
    assert_eq!(status, 200, "{manifest}");
    let reepoched = manifest["data"]["stream_epoch"]
        .as_str()
        .expect("epoch")
        .to_string();
    assert_ne!(reepoched, epoch, "the hand copy is a new stream");
    assert_ne!(reepoched, epoch_before, "not the backup's own epoch either");
    let identity: Value = serde_json::from_slice(
        &std::fs::read(hub.data_dir().join("replica/identity.json")).expect("identity"),
    )
    .expect("json");
    assert_eq!(
        identity["epochs"]
            .as_array()
            .expect("epochs")
            .last()
            .expect("last")["reason"],
        json!("replaced")
    );
    let (bytes, redaction): (Option<String>, Option<String>) = hub
        .db()
        .query_row(
            "SELECT bytes, redaction FROM replica_payload WHERE digest = ?1",
            [&erased_digest],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the barrier row");
    assert_eq!((bytes, redaction.as_deref()), (None, Some("erased")));
    let (_, changes) = hub
        .engine()
        .get("/api/v1/sync/replica/changes?since=0&limit=500");
    assert!(
        !changes.to_string().contains(TOKEN),
        "the feed serves no erased bytes"
    );
}

/// A copy of the backup at `dir` whose database uses 8192-byte pages.
fn other_page_size(dir: &Path, into: &Path) -> PathBuf {
    std::fs::create_dir_all(into).expect("dir");
    let db = into.join("comemory.db");
    std::fs::copy(dir.join("comemory.db"), &db).expect("copy");
    let conn = comemory::store::connection::open(&db).expect("open the copy");
    conn.execute_batch("PRAGMA journal_mode = DELETE; PRAGMA page_size = 8192; VACUUM;")
        .expect("re-page");
    db
}

#[test]
fn restore_without_a_manifest_fails_closed() {
    let hub = Hub::start();
    let a = approved_client(&hub);
    let erased = a.save(
        &format!("A decision the hub erases before its manifest goes missing: {TOKEN}."),
        REPO,
    );
    a.sync();
    let backups = tempfile::tempdir().expect("backups");
    let dir = backups.path().join("hub");
    hub_cli(
        &hub,
        &["backup", "create", "--out", dir.to_str().expect("utf8")],
    );
    hub_cli(&hub, &["erase", "--memory", &erased, "--confirm"]);
    let (epoch_before, _) = identity_of(&hub);

    // A snapshot with another page size is refused, naming both files.
    let other = backups.path().join("other-pages");
    let other_db = other_page_size(&dir, &other);
    let (code, _, stderr) = replica_support::cli_raw(
        &hub.data_dir(),
        &[
            "backup",
            "restore",
            other.to_str().expect("utf8"),
            "--confirm",
        ],
    );
    assert_eq!(code, 64, "{stderr}");
    assert!(
        stderr.contains(other_db.to_str().expect("utf8"))
            && stderr.contains(hub.data_dir().join("comemory.db").to_str().expect("utf8")),
        "{stderr}"
    );
    assert_eq!(identity_of(&hub).0, epoch_before, "nothing changed");

    let manifest = hub.data_dir().join("replica/erasures.jsonl");
    let moved = backups.path().join("erasures.jsonl");
    std::fs::rename(&manifest, &moved).expect("move the manifest away");

    let restored = hub_cli(
        &hub,
        &[
            "backup",
            "restore",
            dir.to_str().expect("utf8"),
            "--confirm",
        ],
    );

    assert_eq!(
        restored["restore_state"],
        json!("erasure_unknown"),
        "{restored}"
    );
    assert!(
        hub_memory_ids(&hub).contains(&erased),
        "local-only: the erase is unknown to the restored hub"
    );
    for (what, answer) in every_sync_route(&hub) {
        assert_refused(&what, answer);
    }
    let (code, _, stderr) = replica_support::cli_raw(&hub.data_dir(), &["rebuild"]);
    assert_eq!(code, 0, "rebuild: {stderr}");
    for (what, answer) in every_sync_route(&hub) {
        assert_refused(&format!("{what} after a rebuild"), answer);
    }

    // The client's exchange records the refusal and sends nothing.
    let owed = save_pending(&a, "An edit the client holds while the hub is unverified.");
    let imports_before = hub.proxy.requests_to("/sync/replica/import").len();
    let (code, stdout, _) = a.cli_raw(&["sync"], &[]);
    assert_eq!(code, 69, "the run ends on the network: {stdout}");
    let status = a.exchange_status();
    assert_eq!(status["network"], json!("restore_unverified"), "{status}");
    assert_eq!(status["outbox"]["pending"], json!(1), "{status}");
    assert_eq!(
        hub.proxy.requests_to("/sync/replica/import").len(),
        imports_before,
        "nothing was pushed"
    );
    assert!(!hub.feed().iter().any(|(_, key, _)| key == &owed));

    let merged = hub_cli(
        &hub,
        &["backup", "merge-erasures", moved.to_str().expect("utf8")],
    );

    assert_eq!(merged["cleared"], json!("erasure_unknown"), "{merged}");
    assert!(manifest.exists(), "the manifest is back in place");
    let (status, body) = hub.engine().get("/api/v1/sync/replica/manifest");
    assert_eq!(status, 200, "{body}");
    assert!(
        !hub_memory_ids(&hub).contains(&erased),
        "merged: erased again"
    );
    // The rebuild ran over the local-only restore, so its own rollback
    // snapshot holds the pre-merge state too: a named snapshot, the
    // operator's to delete, like the ones the restore kept.
    let mut snapshots = kept(&restored);
    snapshots.push(hub.data_dir().join("comemory.db.pre-rebuild.bak"));
    assert_eq!(
        files_holding(&hub.data_dir(), TOKEN, &snapshots),
        Vec::<PathBuf>::new(),
        "no erased content outside the named rollback snapshots"
    );
    let synced = a.sync();
    assert_eq!(
        synced["exchange"]["rebootstrapped"],
        json!(true),
        "{synced}"
    );
    assert!(hub.feed().iter().any(|(_, key, _)| key == &owed));
    let status = a.exchange_status();
    assert_eq!(status["caught_up"], json!(true), "{status}");
}
