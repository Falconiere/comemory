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
//!
//! Also #256 B-7 / AC-10: a policy-loaded client's pulled code generation and
//! pulled document, hidden by revocation and shown again by reapproval.

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

/// Percent-encode the `/` in a repo label for a query string.
fn urlencoding_lite(label: &str) -> String {
    label.replace('/', "%2F")
}

/// The `repos` row for `REPO` on `client`, or `Value::Null` when it is not
/// listed at all — which is what full revocation with no local checkout
/// looks like.
fn repo_row(client: &Client) -> Value {
    client.cli(&["repos", "--repo", REPO])["repos"]
        .as_array()
        .expect("repos")
        .first()
        .cloned()
        .unwrap_or(Value::Null)
}

/// How many edges the code graph answers for `REPO` on `client`.
fn graph_edge_count(client: &Client) -> usize {
    let engine = client.serve();
    let (status, body) = engine.get(&format!("/api/v1/graph?repo={}", urlencoding_lite(REPO)));
    assert_eq!(status, 200, "{body}");
    body["data"]["edges"].as_array().map_or(0, Vec::len)
}

/// A word the shared guides and the client's own local note both use.
const TERM: &str = "workspace";

/// Every `search --only document` hit on `client` for [`TERM`].
fn document_hits(client: &Client) -> Vec<Value> {
    client.cli(&["search", "--only", "document", TERM])["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// `(entity_kind, repository)` for every `policy`-held pull position on
/// `client`, oldest first.
fn policy_holds(client: &Client) -> Vec<(String, Option<String>)> {
    let conn = client.open();
    let mut statement = conn
        .prepare(
            "SELECT entity_kind, repository FROM replica_pull_hold \
              WHERE reason = 'policy' ORDER BY from_sequence",
        )
        .expect("prepare");
    statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect")
}

/// #256 B-7 / AC-10: a client with a loaded policy pulls a code generation
/// and a document of `REPO` from a real hub. Revoking `REPO` hides both from
/// `repos`, the code graph and `search --only document`, while the client's
/// own local document index of `REPO` keeps answering. A later revision
/// pulled while still revoked is held, not dropped; reapproval shows the
/// pulled cache again, now at that held revision.
#[test]
fn revocation_hides_pulled_caches_only() {
    let hub = Hub::start();
    let a = approved_client(&hub);
    let workdir = tempfile::tempdir().expect("workdir");

    // `a` builds a real checkout and a real docs tree under REPO, and pushes
    // both to the hub.
    let code_root = replica_support::pinned_repo(workdir.path(), 12);
    replica_support::git(
        &code_root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/falconiere/comemory.git",
        ],
    );
    a.cli(&[
        "index-code",
        "--repo",
        REPO,
        "--path",
        code_root.to_str().expect("utf8 path"),
    ]);
    // Documents share only from under the label's indexed root
    // (`repo_marker.root_path`, set by `index-code` above), so the guides
    // land INSIDE the same checkout `code_root` names, not a sibling of it.
    let docs_root = replica_support::docs_tree(workdir.path(), "pinned-repo");
    assert_eq!(docs_root, code_root, "the premise: one root, two indexers");
    let guides_dir = docs_root.join("docs/guides");
    a.cli(&[
        "index",
        guides_dir.to_str().expect("utf8 path"),
        "--repo",
        REPO,
    ]);
    a.sync();
    let head_1 = repo_row(&a)["last_head"]
        .as_str()
        .expect("a's first head")
        .to_string();

    // `b`: a client with a loaded policy, approved for REPO, with its own
    // small local document index of REPO under a path it never shares with
    // anyone — a real local index, untouched by anything about the cache.
    let b = approved_client(&hub);
    let b_notes = workdir.path().join("b-notes");
    std::fs::create_dir_all(&b_notes).expect("b notes dir");
    std::fs::write(
        b_notes.join("local-notes.md"),
        // n=50 is a real excerpt that happens to use TERM, same as the
        // shared guides do.
        format!("# Local notes\n\n{}", exchange_support::guide_body(50)),
    )
    .expect("write local note");
    b.cli(&[
        "index",
        b_notes.to_str().expect("utf8 path"),
        "--repo",
        REPO,
    ]);
    b.sync();

    let before = repo_row(&b);
    assert_eq!(
        before["shared_head"],
        json!(head_1),
        "the pulled generation shows in repos: {before}"
    );
    assert!(
        graph_edge_count(&b) > 0,
        "and the pulled edges answer the graph"
    );
    let hits_before = document_hits(&b);
    assert!(
        hits_before.iter().any(|h| h["shared_from"] == json!(REPO)),
        "the pulled document answers search: {hits_before:?}"
    );
    assert!(
        hits_before.iter().any(|h| h["shared_from"].is_null()),
        "and so does b's own local note: {hits_before:?}"
    );

    // Revoke REPO on `b` — the same store-API write a policy load leaves.
    b.approve(&hub.api_url(), WORKSPACE, &[], 2);

    let revoked = repo_row(&b);
    assert!(
        revoked["shared_head"].is_null(),
        "revoked: the pulled generation is hidden from repos: {revoked}"
    );
    assert_eq!(
        graph_edge_count(&b),
        0,
        "revoked: the pulled edges are hidden from the graph"
    );
    let hits_revoked = document_hits(&b);
    assert!(
        !hits_revoked.iter().any(|h| h["shared_from"] == json!(REPO)),
        "revoked: the pulled document is hidden from search: {hits_revoked:?}"
    );
    assert!(
        hits_revoked.iter().any(|h| h["shared_from"].is_null()),
        "but b's own local note keeps answering: {hits_revoked:?}"
    );
    let cache_rows: i64 = b
        .open()
        .query_row("SELECT COUNT(*) FROM remote_code_edge", [], |r| r.get(0))
        .expect("count");
    assert!(cache_rows > 0, "hidden, not deleted");

    // `a` moves to a second revision of both, while `b` stays revoked.
    std::fs::write(code_root.join("file_9999.rs"), "pub fn added_later() {}\n").expect("write");
    replica_support::git(&code_root, &["add", "-A"]);
    replica_support::git(&code_root, &["commit", "-q", "-m", "add a file"]);
    a.cli(&[
        "index-code",
        "--repo",
        REPO,
        "--path",
        code_root.to_str().expect("utf8 path"),
    ]);
    std::fs::write(
        guides_dir.join("extra-guide.md"),
        format!("# Extra\n\n{}", exchange_support::guide_body(1)),
    )
    .expect("write extra guide");
    a.cli(&[
        "index",
        guides_dir.to_str().expect("utf8 path"),
        "--repo",
        REPO,
    ]);
    a.sync();
    let head_2 = repo_row(&a)["last_head"]
        .as_str()
        .expect("a's second head")
        .to_string();
    assert_ne!(head_2, head_1, "the premise: a genuinely new generation");

    // `b` pulls again while still revoked: the new entries are held, not
    // dropped and not applied (#255's existing hold mechanism).
    b.sync();
    let held = policy_holds(&b);
    assert!(
        held.iter().any(|(k, _)| k == "code_generation"),
        "the second code generation is held: {held:?}"
    );
    assert!(
        held.iter().any(|(k, _)| k == "document_revision"),
        "the new document revision is held too: {held:?}"
    );
    assert!(
        repo_row(&b)["shared_head"].is_null(),
        "still revoked, still hidden"
    );

    // Reapprove REPO: the held entries apply.
    b.approve(&hub.api_url(), WORKSPACE, &[REPO], 3);
    b.sync();

    let reapproved = repo_row(&b);
    assert_eq!(
        reapproved["shared_head"],
        json!(head_2),
        "the held generation applied on reapproval: {reapproved}"
    );
    assert!(
        graph_edge_count(&b) > 0,
        "and the graph answers from it again"
    );
    let hits_after = document_hits(&b);
    assert!(
        hits_after.iter().any(|h| h["shared_from"] == json!(REPO)),
        "the held document applied too: {hits_after:?}"
    );
}

/// Env var that arms candidate capture for one `comemory find` run.
const CAPTURE_ON: (&str, &str) = ("COMEMORY_OBSERVATIONS_ENABLED", "1");

/// Every `replica_*` table and column that holds `marker`, as `table.column`.
/// A generic sweep rather than a named list, so a future replica table needs
/// no update here to stay covered.
fn replica_columns_holding(conn: &rusqlite::Connection, marker: &str) -> Vec<String> {
    let mut tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'replica_%'")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<std::result::Result<_, _>>()
        .expect("collect");
    tables.sort();
    let mut hits = Vec::new();
    for table in tables {
        let columns: Vec<String> = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("prepare")
            .query_map([], |r| r.get(1))
            .expect("query")
            .collect::<std::result::Result<_, _>>()
            .expect("collect");
        for column in columns {
            let count: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table} \
                          WHERE instr(CAST({column} AS TEXT), ?1) > 0"
                    ),
                    [marker],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if count > 0 {
                hits.push(format!("{table}.{column}"));
            }
        }
    }
    hits
}

/// A word only in the memory body — legitimately replicated as the memory's
/// own payload, so it proves the pool actually captured the passage rather
/// than proving anything about a leak.
const CMARK: &str = "candidateProbeVexillum4407";

/// A word only in the query string — never part of any memory, document or
/// code content, so its presence anywhere is unambiguously the captured
/// query, not something a legitimate push was always going to carry.
const QMARK: &str = "queryProbeVexillum9214";

/// #256 B-7 / AC-10: a captured candidate observation never reaches a
/// replica table or an outbound push body, whether captured before or
/// redacted after a purge.
#[test]
fn candidate_observations_never_leak_into_replica_state_or_a_push_body() {
    let hub = Hub::start();
    let a = approved_client(&hub);
    let id = a.save(
        &format!("A decision naming {CMARK}, for a search to surface."),
        REPO,
    );
    a.sync();

    let (code, stdout, stderr) = a.cli_raw(&["find", &format!("{CMARK} {QMARK}")], &[CAPTURE_ON]);
    assert_eq!(code, 0, "find: {stderr}");
    let found: Value = serde_json::from_str(stdout.trim()).expect("find json");
    assert!(
        found["observation_id"].is_string(),
        "the premise: capture was armed: {found}"
    );

    let conn = a.open();
    let query_captured: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM candidate_query_observations WHERE instr(query, ?1) > 0",
            [QMARK],
            |r| r.get(0),
        )
        .expect("count");
    assert!(
        query_captured > 0,
        "the premise: the query was captured verbatim"
    );
    let passage_captured: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM candidate_observations WHERE instr(text, ?1) > 0",
            [CMARK],
            |r| r.get(0),
        )
        .expect("count");
    assert!(
        passage_captured > 0,
        "the premise: the memory's own passage was pooled"
    );
    assert_eq!(
        replica_columns_holding(&conn, QMARK),
        Vec::<String>::new(),
        "captured: no replica table carries the captured query"
    );
    drop(conn);

    a.sync();
    assert!(
        !hub.proxy.log().iter().any(|l| l.body.contains(QMARK)),
        "captured: no push body ever carried the captured query: {:?}",
        hub.proxy.log()
    );

    a.cli(&["erase", "--memory", &id, "--confirm"]);

    let conn = a.open();
    let redacted: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM candidate_observations WHERE instr(text, ?1) > 0",
            [CMARK],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(redacted, 0, "the purge redacted the candidate's passage");
    assert_eq!(
        replica_columns_holding(&conn, QMARK),
        Vec::<String>::new(),
        "redacted: still no replica table carries the captured query"
    );
    drop(conn);

    a.sync();
    assert!(
        !hub.proxy.log().iter().any(|l| l.body.contains(QMARK)),
        "redacted: still no push body carries the captured query: {:?}",
        hub.proxy.log()
    );
}
